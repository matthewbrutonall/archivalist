# Archivalist

Archivalist is an independent, open-source archival management project derived
from the [ArchivesSpace](https://archivesspace.org/) codebase.

The project preserves the proven archival-management foundation of ArchivesSpace
while progressively re-engineering performance-critical components in Rust. This
migration is being carried out incrementally, with compatibility tests, feature
flags, parity gates, and scoped fallback paths used to reduce migration risk.

Archivalist's ambitions extend beyond technical modernisation. Its long-term goal
is to provide a platform designed for AI-assisted archival work and first-class
[IIIF](https://iiif.io/) interoperability.

> [!WARNING]
> Archivalist is under active development. It is not yet recommended for
> production use or as a drop-in replacement for an existing ArchivesSpace
> installation.

## Why Archivalist?

Archives need systems that can manage established descriptive practice while
also responding to rapidly growing digital collections, computational workflows,
and new expectations for online access. Archivalist is being developed to:

- improve performance and scalability by moving suitable workloads to
  memory-safe Rust services;
- modernise the architecture without attempting a risky all-at-once rewrite;
- retain open formats, standards, and data portability;
- support AI-assisted description, metadata enrichment, discovery, and workflow
  automation while keeping professional archivists in control of decisions;
- make the International Image Interoperability Framework (IIIF), including
  manifests, image services, viewers, and annotations, a first-class part of
  managing and presenting digital collections; and
- provide a sustainable foundation for continued experimentation in archival
  technology.

## Current architecture

Archivalist currently has a hybrid architecture. The inherited application and
API remain authoritative, while selected operations are moved behind a Rust
sidecar only after they pass contract, parity, performance, and fallback tests.
The Ruby indexer remains responsible for scheduling work and writing to the
primary Solr collection.

This approach allows the project to improve the existing system in measurable
stages. Migrated paths that currently support fallback return to their native
implementation when the corresponding Rust service is disabled or unavailable.

## Engineering progress

The current Rust workspace provides:

- typed access to the archival-management API;
- validation and performance-profiling tools;
- an experimental, separate shadow-index and query layer;
- feature-gated Rust implementations of selected search and resource-tree
  operations;
- preparation of common staff-index documents and deterministic, non-AI metadata
  enrichment; and
- repeatable canary, parity, benchmark, packaging, and rollback tooling.

These paths are feature-gated and disabled by default. See the
[Rust tools documentation](rust-tools/README.md) for the implemented components,
build instructions, and safety model.

AI-assisted workflows and comprehensive IIIF integration are project goals and
are not presented as completed features in the current development version.

## Roadmap

Archivalist's development priorities are:

1. Continue the evidence-led migration of performance-critical services to Rust.
2. Preserve behavioural and data compatibility through automated parity testing.
3. Develop AI-assisted archival description, enrichment, discovery, and workflow
   tools with clear human review and provenance.
4. Build comprehensive IIIF Presentation API, Image API, viewer, and annotation
   support into the core application.
5. Simplify installation, upgrades, monitoring, and recovery.
6. Establish an independent, well-documented release process suitable for wider
   testing and eventual production use.

## Relationship to ArchivesSpace

Archivalist began from the ArchivesSpace source code and continues to benefit from
the work of the ArchivesSpace community. It is now an independent project with a
separate name, repository, technical direction, and release roadmap. It is not an
official ArchivesSpace release and is not affiliated with or endorsed by the
ArchivesSpace program.

Compatibility with inherited ArchivesSpace behaviour is treated as an engineering
baseline during the migration, not as a permanent restriction on Archivalist's
design.

## Development

The project is currently intended for developers and controlled test environments.
The existing [Docker development guide](DOCKER_DEV.md) describes the inherited
development stack. Rust-specific build and test instructions are in
[rust-tools/README.md](rust-tools/README.md).

Production installation and migration instructions will be published when the
software reaches an appropriate release stage.

## Contributing and issues

Contributions, design discussion, test results, and bug reports are welcome.
Please read [CONTRIBUTING.md](CONTRIBUTING.md) and use this repository's
[GitHub issue tracker](https://github.com/matthewbrutonall/archivalist/issues) for
Archivalist-specific reports and proposals.

## Licence

Archivalist is distributed under the
[Educational Community License, version 2.0](https://opensource.org/license/ecl-2-0/).
See [COPYING](COPYING) for the full licence text.

## Acknowledgements

Archivalist is derived from ArchivesSpace, the community-supported open-source
archives information management application. The project acknowledges the
ArchivesSpace community and the many institutions, funders, developers,
archivists, and other contributors whose work created and continues to sustain
the original codebase.

ArchivesSpace 1.0 was developed by Hudson Molonglo in partnership with New York
University Libraries, UC San Diego Libraries, and the University of Illinois
Urbana-Champaign Library, with funding from the Andrew W. Mellon Foundation and
organisational support from Lyrasis.
