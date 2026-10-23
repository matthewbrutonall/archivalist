#!/usr/bin/env python3
"""Disposable profiling stack: setup, verify, status, teardown.

The full-round native indexing profile and the large-repository seeder must never touch a main or
shared test database. This script provisions a SEPARATE stack next to the existing app container
and proves, before any tool writes or indexes, that a backend is really connected to it.

setup     Starts NEW containers only (it never restarts or modifies existing ones):
            archivalist-profiling-db-<id>       MySQL on a new named volume (no published port)
            archivalist-profiling-solr-<id>     Solr with the repository's configset, an
                                                'archivesspace' core and an identity core
                                                archivalist_profiling_identity_<id>
            archivalist-profiling-backend-<id>  the app container's image with the source checkout
                                                mounted READ-ONLY, its own build volume (seeded by
                                                a one-shot container that sees the app's mounts
                                                read-only) and its own data volume; it runs the
                                                db:migrate and backend:devserver commands directly
                                                (no Ant targets, so nothing writes to the source)
                                                against the profiling database only
          Both created containers' mounts are checked with docker inspect before they start:
          setup fails if any writable mount is shared with the app or is not this stack's own
          volume. All carry the label archivalist.profiling-stack=<id>, join the app container's
          user-defined network under fixed aliases (archivalist-profiling-{db,solr,backend}) and
          publish ports on 127.0.0.1 only. It then creates one identity repository in the new
          database and writes a state file (mode 0600) under rust-tools/benchmarks/.

verify    Challenge-response proof of database identity. A fresh random nonce is written
          DIRECTLY into the profiling database (docker exec into the labelled MySQL container)
          as the identity repository's name; the target backend must then return that nonce
          from an anonymous GET /repositories/<id>. A backend on any other database cannot,
          and nothing is ever written through the target backend (the check is a single GET).
          Solr is accepted only if its admin API reports the stack's identity core. Prints the
          nonce for the tools, which re-check it from their own network location.

status    Shows the labelled containers and the volume.

teardown  Removes only containers carrying this stack's label (double-checked per container).
          The stack's db, build and data volumes are preserved unless --delete-volumes is given.

Only the Python standard library and the docker CLI are used.
"""

import argparse
import json
import os
import secrets
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
STATE_DIR = os.path.join(REPO_ROOT, "rust-tools", "benchmarks")
LABEL = "archivalist.profiling-stack"
ROLE_LABEL = "archivalist.profiling-role"
ALIASES = {"db": "archivalist-profiling-db", "solr": "archivalist-profiling-solr", "backend": "archivalist-profiling-backend"}
IDENTITY_NAME_PREFIX = "archivalist profiling identity "

# The commands build.xml's db:migrate and backend:devserver run, invoked directly so that nothing is
# written into the read-only source checkout (backend:copy:templates would). Same JVM options.
BACKEND_SCRIPT = r"""set -euo pipefail
JAR=$(ls /archivesspace/build/jruby-complete-*.jar | head -1)
GEMS=$(ls -d /archivesspace/build/gems/jruby/*/ | head -1)
CP="$JAR:${GEMS%/}:/archivesspace/lib/*:/archivesspace/common:/archivesspace/common/lib/*"
export GEM_HOME=/archivesspace/build/gems GEM_PATH= BUNDLE_PATH=/archivesspace/build/gems BUNDLE_GEMFILE= ASPACE_ENV=development
OPTS="-Djruby.rack.response.swallow_client_abort=false -Djava.security.egd=file:/dev/./urandom -Xmx2048m -Xss2m -Dfile.encoding=UTF-8 \
 -Daspace.config.search_user_secret=devserver -Daspace.config.public_user_secret=devserver -Daspace.config.staff_user_secret=devserver \
 -Daspace.devserver=true -Daspace.config.frontend_cookie_secret=devserver -Daspace.config.public_cookie_secret=devserver \
 --add-opens java.base/sun.nio.ch=ALL-UNNAMED --add-opens java.base/java.io=ALL-UNNAMED"
cd /archivesspace/build
java $OPTS ${JAVA_OPTS:-} -cp "$CP" org.jruby.Main -Iapp/lib ../build/scripts/migrate_db.rb
cd /archivesspace/backend
exec java -Daspace.service=backend -Daspace.config.backend_url=http://localhost:4567 -Daspace.config.ignore_schema_info_check=true \
  $OPTS ${JAVA_OPTS:-} -Djruby.debug.fullTrace=true -cp "$CP" org.jruby.Main app/main.rb 4567
"""


class StackError(Exception):
    pass


def docker(*args, check=True, env=None, input_text=None):
    result = subprocess.run(["docker", *args], capture_output=True, text=True, env=env, input=input_text)
    if check and result.returncode != 0:
        raise StackError(f"docker {' '.join(args[:3])} ... failed: {result.stderr.strip()[:500]}")
    return result


def inspect(name):
    """Container details, or None. Containers only: a volume may share the database container's name."""
    result = docker("container", "inspect", name, check=False)
    if result.returncode != 0:
        return None
    return json.loads(result.stdout)[0]


def http_json(url, timeout=10):
    with urllib.request.urlopen(url, timeout=timeout) as resp:
        return json.loads(resp.read())


def port_free(port):
    with socket.socket() as sock:
        return sock.connect_ex(("127.0.0.1", port)) != 0


def wait_for(description, predicate, timeout_s, interval_s=3):
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        try:
            if predicate():
                return
        except Exception:
            pass
        time.sleep(interval_s)
    raise StackError(f"timed out after {timeout_s}s waiting for {description}")


def app_container_details(app_container):
    info = inspect(app_container)
    if info is None:
        raise StackError(f"app container {app_container!r} not found")
    if not info["State"].get("Running"):
        raise StackError(f"app container {app_container!r} is not running")
    networks = [n for n in info["NetworkSettings"]["Networks"] if n not in ("bridge", "host", "none")]
    if not networks:
        raise StackError("app container is not on a user-defined network; network aliases are unavailable")
    mounts = [m for m in info.get("Mounts", []) if m["Type"] in ("bind", "volume")]
    sources = [m for m in mounts if m["Destination"] == "/archivesspace"]
    if len(sources) != 1:
        raise StackError("app container must have exactly one mount at /archivesspace (the source checkout)")
    env = dict(e.split("=", 1) for e in info["Config"].get("Env", []) if "=" in e)
    return {"image": info["Config"]["Image"] or info["Image"], "network": networks[0], "mounts": mounts,
            "source_mount": sources[0], "shared": {mount_identity(m) for m in mounts},
            "java_opts": env.get("JAVA_OPTS", "")}


def mount_identity(m):
    """What a mount actually shares: the host path of a bind or the name of a volume."""
    return ("bind", m["Source"]) if m["Type"] == "bind" else ("volume", m["Name"])


def mount_spec(m, destination=None, readonly=False):
    dst = destination or m["Destination"]
    spec = (f"type=bind,src={m['Source']},dst={dst}" if m["Type"] == "bind"
            else f"type=volume,src={m['Name']},dst={dst}")
    return spec + (",readonly" if readonly else "")


def verify_isolated_mounts(container, app_shared, own_volumes):
    """Inspects a created container's mounts. Every writable mount must be one of this stack's own
    volumes and none may be shared with the app container; returns a summary for the state file."""
    info = inspect(container)
    if info is None:
        raise StackError(f"cannot inspect {container}")
    summary = []
    for m in info.get("Mounts", []):
        ident = mount_identity(m)
        writable = bool(m.get("RW", True))
        summary.append({"type": m["Type"], "source": ident[1], "destination": m["Destination"], "rw": writable})
        if writable and ident in app_shared:
            raise StackError(f"{container}: writable mount {ident} is shared with the main app")
        if writable and not (m["Type"] == "volume" and m.get("Name") in own_volumes):
            raise StackError(f"{container}: writable mount {ident} is not one of this stack's own volumes")
    return summary


def existing_stacks():
    result = docker("ps", "-a", "--filter", f"label={LABEL}", "--format", "{{.Names}}")
    return [n for n in result.stdout.split() if n]


def mysql_exec(state, sql):
    """Runs SQL as root inside the labelled profiling MySQL container (password via env, not args)."""
    container = state["containers"]["db"]
    require_labelled(container, state["stack_id"], "db")
    env = dict(os.environ, MYSQL_PWD=state["db_root_password"])
    result = docker("exec", "-e", "MYSQL_PWD", container, "mysql", "-uroot", "-N", "-B", "-e", sql, env=env)
    return result.stdout.strip()


def require_labelled(container, stack_id, role):
    info = inspect(container)
    if info is None:
        raise StackError(f"stack container {container} does not exist")
    labels = info["Config"].get("Labels") or {}
    if labels.get(LABEL) != stack_id or labels.get(ROLE_LABEL) != role:
        raise StackError(f"container {container} is not this stack's {role} (labels {labels})")
    if not info["State"].get("Running"):
        raise StackError(f"stack container {container} is not running")
    return info


def load_state(path):
    with open(path) as fh:
        state = json.load(fh)
    for key in ("stack_id", "containers", "db_root_password", "identity_repo_id", "identity_repo_code",
                "backend_host_url", "solr_host_url", "identity_core"):
        if key not in state:
            raise StackError(f"state file {path} lacks {key}")
    return state


def setup(args):
    if existing_stacks():
        raise StackError(f"profiling stack containers already exist: {existing_stacks()}; tear it down first")
    for port in (args.backend_port, args.solr_port):
        if not port_free(port):
            raise StackError(f"127.0.0.1:{port} is already in use")
    app = app_container_details(args.app_container)
    stack_id = secrets.token_hex(4)
    names = {role: f"{alias}-{stack_id}" for role, alias in ALIASES.items()}
    volumes = {role: f"archivalist-profiling-{role}-{stack_id}" for role in ("db", "build", "data")}
    volume = volumes["db"]
    db_root_password = secrets.token_hex(16)
    db_password = secrets.token_hex(16)
    labels = ["--label", f"{LABEL}={stack_id}"]
    print(f"Stack {stack_id}: app image {app['image']}, network {app['network']}", flush=True)

    docker("volume", "create", "--label", f"{LABEL}={stack_id}", volume)
    docker("run", "-d", "--name", names["db"], *labels, "--label", f"{ROLE_LABEL}=db",
           "--network", app["network"], "--network-alias", ALIASES["db"],
           "-e", f"MYSQL_ROOT_PASSWORD={db_root_password}", "-e", "MYSQL_DATABASE=archivesspace",
           "-e", "MYSQL_USER=as", "-e", f"MYSQL_PASSWORD={db_password}",
           "--mount", f"type=volume,src={volume},dst=/var/lib/mysql",
           args.mysql_image, "--log-bin-trust-function-creators=1",
           "--character-set-server=utf8mb4", "--collation-server=utf8mb4_unicode_ci")
    docker("run", "-d", "--name", names["solr"], *labels, "--label", f"{ROLE_LABEL}=solr",
           "--network", app["network"], "--network-alias", ALIASES["solr"],
           "-p", f"127.0.0.1:{args.solr_port}:8983", "-e", "SOLR_MODULES=analysis-extras", args.solr_image)

    state = {"stack_id": stack_id, "created_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
             "app_container": args.app_container, "network": app["network"], "image": app["image"],
             "containers": names, "db_volume": volume, "volumes": volumes, "db_root_password": db_root_password,
             "backend_host_url": f"http://127.0.0.1:{args.backend_port}",
             "backend_network_url": f"http://{ALIASES['backend']}:4567",
             "solr_host_url": f"http://127.0.0.1:{args.solr_port}/solr",
             "solr_network_url": f"http://{ALIASES['solr']}:8983/solr",
             "identity_core": f"archivalist_profiling_identity_{stack_id}",
             "admin_user": "admin", "admin_password": "admin"}

    wait_for("profiling MySQL", lambda: mysql_exec({**state, "identity_repo_id": 0}, "SELECT 1") == "1", 180)
    print("MySQL ready", flush=True)

    wait_for("profiling Solr", lambda: http_json(f"{state['solr_host_url']}/admin/cores?action=STATUS&wt=json") is not None, 180)
    docker("cp", os.path.join(REPO_ROOT, "solr"), f"{names['solr']}:/tmp/archivesspace-solr-conf")
    docker("exec", "-u", "0", names["solr"], "bash", "-c",
           "mkdir -p /var/solr/data/configsets/archivesspace/conf && "
           "cp /tmp/archivesspace-solr-conf/* /var/solr/data/configsets/archivesspace/conf/ && "
           "chown -R solr:solr /var/solr/data/configsets")
    for core in ("archivesspace", state["identity_core"]):
        http_json(f"{state['solr_host_url']}/admin/cores?" +
                  urllib.parse.urlencode({"action": "CREATE", "name": core, "configSet": "archivesspace", "wt": "json"}), timeout=120)
    print(f"Solr ready with cores archivesspace and {state['identity_core']}", flush=True)

    # Own build and data volumes. build/ is seeded by a one-shot container that sees the app's
    # mounts READ-ONLY; the backend then mounts only the source (read-only) plus its own volumes.
    for vol in (volumes["build"], volumes["data"]):
        docker("volume", "create", "--label", f"{LABEL}={stack_id}", vol)
    own_volumes = set(volumes.values())
    seed = f"archivalist-profiling-seed-{stack_id}"
    seed_mounts = [a for m in app["mounts"] for a in ("--mount", mount_spec(m, readonly=True))]
    docker("create", "--name", seed, *labels, "--label", f"{ROLE_LABEL}=seed", *seed_mounts,
           "--mount", f"type=volume,src={volumes['build']},dst=/profiling-build",
           "--entrypoint", "bash", app["image"], "-c", "cp -a /archivesspace/build/. /profiling-build/")
    try:
        state["mounts"] = {"seed": verify_isolated_mounts(seed, app["shared"], own_volumes)}
        docker("start", "-a", seed)
        exit_code = inspect(seed)["State"]["ExitCode"]
        if exit_code != 0:
            raise StackError(f"seeding the build volume failed (exit {exit_code})")
    finally:
        docker("rm", "-f", seed, check=False)
    print(f"Seeded build volume {volumes['build']} from the app's build directory (read-only)", flush=True)

    db_url = (f"jdbc:mysql://{ALIASES['db']}:3306/archivesspace?useUnicode=true&characterEncoding=UTF-8"
              f"&user=as&password={db_password}&useSSL=false&allowPublicKeyRetrieval=true")
    docker("create", "--name", names["backend"], *labels, "--label", f"{ROLE_LABEL}=backend",
           "--network", app["network"], "--network-alias", ALIASES["backend"],
           "-p", f"127.0.0.1:{args.backend_port}:4567",
           "--mount", mount_spec(app["source_mount"], "/archivesspace", readonly=True),
           "--mount", f"type=volume,src={volumes['build']},dst=/archivesspace/build",
           "--mount", f"type=volume,src={volumes['data']},dst=/archivalist-profiling-data",
           "-e", f"APPCONFIG_DB_URL={db_url}", "-e", f"APPCONFIG_SOLR_URL={state['solr_network_url']}/archivesspace",
           "-e", "APPCONFIG_DATA_DIRECTORY=/archivalist-profiling-data",
           "-e", "APPCONFIG_SEARCH_USER_SECRET=devserver", "-e", f"JAVA_OPTS={app['java_opts']}",
           "--entrypoint", "bash", app["image"], "-c", BACKEND_SCRIPT)
    try:
        state["mounts"]["backend"] = verify_isolated_mounts(names["backend"], app["shared"], own_volumes)
    except StackError:
        docker("rm", "-f", names["backend"], check=False)
        raise
    docker("start", names["backend"])
    print("Backend container verified (source read-only, own build/data volumes) and started; "
          "migrating and starting (this can take several minutes)", flush=True)
    wait_for("profiling backend", lambda: "databaseProductName" in http_json(state["backend_host_url"] + "/"), args.backend_timeout, 5)

    login = urllib.request.Request(f"{state['backend_host_url']}/users/admin/login?password=admin", data=b"", method="POST")
    with urllib.request.urlopen(login, timeout=30) as resp:
        session = json.loads(resp.read())["session"]
    repo_code = f"ARCHPROF-{stack_id}"
    req = urllib.request.Request(f"{state['backend_host_url']}/repositories", method="POST",
                                 data=json.dumps({"repo_code": repo_code, "name": IDENTITY_NAME_PREFIX + "initial"}).encode(),
                                 headers={"Content-Type": "application/json", "X-ArchivesSpace-Session": session})
    with urllib.request.urlopen(req, timeout=30) as resp:
        repo_id = json.loads(resp.read())["id"]
    state.update(identity_repo_id=repo_id, identity_repo_code=repo_code)

    os.makedirs(STATE_DIR, exist_ok=True)
    path = args.state or os.path.join(STATE_DIR, f"profiling_stack_{stack_id}.json")
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "w") as fh:
        json.dump(state, fh, indent=2)
    print(f"State file: {path}", flush=True)
    nonce = verify_identity(state, state["backend_host_url"], state["solr_host_url"])
    print(f"Verified: backend and Solr belong to stack {stack_id} (nonce {nonce})")
    return 0


def verify_identity(state, backend_url, solr_url):
    """Returns a fresh nonce proven to be readable through backend_url; raises otherwise."""
    for role in ("db", "solr", "backend"):
        require_labelled(state["containers"][role], state["stack_id"], role)
    nonce = secrets.token_hex(16)
    repo_id = int(state["identity_repo_id"])
    code = state["identity_repo_code"]
    if not all(c.isalnum() or c == "-" for c in code):
        raise StackError("unexpected identity repo code")
    changed = mysql_exec(state, f"UPDATE archivesspace.repository SET name='{IDENTITY_NAME_PREFIX}{nonce}' "
                                f"WHERE id={repo_id} AND repo_code='{code}'; SELECT ROW_COUNT();")
    if changed.splitlines()[-1].strip() != "1":
        raise StackError(f"identity record not found in the profiling database (ROW_COUNT {changed!r})")
    try:
        seen = http_json(f"{backend_url.rstrip('/')}/repositories/{repo_id}")
    except (urllib.error.URLError, ValueError) as err:
        raise StackError(f"backend {backend_url} did not return the identity record: {err}") from err
    if seen.get("name") != IDENTITY_NAME_PREFIX + nonce or seen.get("repo_code") != code:
        raise StackError(f"backend {backend_url} is NOT connected to profiling stack {state['stack_id']} "
                         f"(identity mismatch: got repo_code={seen.get('repo_code')!r})")
    try:
        status = http_json(f"{solr_url.rstrip('/')}/admin/cores?action=STATUS&core={state['identity_core']}&wt=json")
    except (urllib.error.URLError, ValueError) as err:
        raise StackError(f"Solr {solr_url} unreachable: {err}") from err
    if state["identity_core"] not in (status.get("status") or {}) or not status["status"][state["identity_core"]]:
        raise StackError(f"Solr {solr_url} is NOT the profiling stack's Solr (identity core missing)")
    return nonce


def verify(args):
    state = load_state(args.state)
    backend_url = args.backend_url or state["backend_host_url"]
    solr_url = args.solr_url or state["solr_host_url"]
    nonce = verify_identity(state, backend_url, solr_url)
    if args.emit_env:
        print(f"ARCHIVALIST_PROFILING_STACK_ID={state['stack_id']}")
        print(f"ARCHIVALIST_PROFILING_STACK_NONCE={nonce}")
        print(f"ARCHIVALIST_PROFILING_STACK_IDENTITY_REPO_ID={state['identity_repo_id']}")
        print(f"ARCHIVALIST_PROFILING_STACK_IDENTITY_CORE={state['identity_core']}")
        print(f"ARCHIVALIST_PROFILING_STACK_BACKEND_URL={state['backend_network_url']}")
        print(f"ARCHIVALIST_PROFILING_STACK_SOLR_URL={state['solr_network_url']}")
    else:
        print(f"Verified: {backend_url} and {solr_url} belong to profiling stack {state['stack_id']}")
    return 0


def status(args):
    state = load_state(args.state)
    for role, name in state["containers"].items():
        info = inspect(name)
        print(f"{role:8} {name:45} {'running' if info and info['State'].get('Running') else 'absent/stopped'}")
    for vol_name in (state.get("volumes") or {"db": state["db_volume"]}).values():
        vol = docker("volume", "inspect", vol_name, check=False)
        print(f"volume   {vol_name:45} {'present' if vol.returncode == 0 else 'absent'}")
    return 0


def teardown(args):
    state = load_state(args.state)
    stack_id = state["stack_id"]
    names = list(state["containers"].values())
    names += [n for n in docker("ps", "-a", "--filter", f"label={LABEL}={stack_id}", "--format", "{{.Names}}").stdout.split()
              if n not in names]
    for name in names:
        info = inspect(name)
        if info is None:
            print(f"{name}: already absent")
            continue
        labels = info["Config"].get("Labels") or {}
        if labels.get(LABEL) != stack_id:
            raise StackError(f"refusing to remove {name}: labels {labels} do not match stack {stack_id}")
        docker("rm", "-f", name)
        print(f"removed container {name}")
    stack_volumes = list((state.get("volumes") or {"db": state["db_volume"]}).values())
    if args.delete_volumes:
        for vol_name in stack_volumes:
            vol = docker("volume", "inspect", vol_name, check=False)
            if vol.returncode != 0:
                continue
            vol_labels = json.loads(vol.stdout)[0].get("Labels") or {}
            if vol_labels.get(LABEL) != stack_id:
                raise StackError(f"refusing to delete volume {vol_name}: not labelled for stack {stack_id}")
            docker("volume", "rm", vol_name)
            print(f"deleted volume {vol_name} (explicitly requested)")
    else:
        print(f"kept volumes {', '.join(stack_volumes)} (pass --delete-volumes to delete them)")
    return 0


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("setup")
    p.add_argument("--app-container", default="archivesspaceai-app-1")
    p.add_argument("--backend-port", type=int, default=14567)
    p.add_argument("--solr-port", type=int, default=18983)
    p.add_argument("--mysql-image", default="mysql:8")
    p.add_argument("--solr-image", default="solr:9.4.1")
    p.add_argument("--backend-timeout", type=int, default=1200)
    p.add_argument("--state", default=None, help="state file path (default rust-tools/benchmarks/profiling_stack_<id>.json)")
    p.set_defaults(func=setup)
    for name, func in (("verify", verify), ("status", status), ("teardown", teardown)):
        p = sub.add_parser(name)
        p.add_argument("--state", required=True)
        if name == "verify":
            p.add_argument("--backend-url", default=None, help="backend to check (default: the stack's host URL)")
            p.add_argument("--solr-url", default=None, help="Solr to check (default: the stack's host URL)")
            p.add_argument("--emit-env", action="store_true", help="print the variables the tools re-check")
        if name == "teardown":
            p.add_argument("--delete-volumes", action="store_true", help="also delete the stack's db/build/data volumes (explicit only)")
        p.set_defaults(func=func)
    args = parser.parse_args(argv)
    try:
        return args.func(args)
    except StackError as err:
        print(f"ERROR: {err}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
