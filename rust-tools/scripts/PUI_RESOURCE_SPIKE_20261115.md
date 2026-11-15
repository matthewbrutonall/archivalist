# One-resource PUI indexing spike (closed)

This local, read-only experiment compares native PUI work on
`/repositories/4/resources/6` with a hybrid that keeps ordinary archival-object
preparation in Ruby and builds ordered/tree auxiliary documents in Rust. It
does not write to Solr, exercise fallback, or alter PUI feature flags.

Five alternating pairs on the isolated profiling backend each matched 2,843
pre-Solr documents type-strictly, including embedded JSON, and the same 1,000
delete candidates. The final release binary's SHA-256 was
`8eb53cba7081e77e212d0988b2de9778143a86aad66b9b94ea971752460f869a`.
The native wall median was 13.384 seconds; the hybrid median was 10.153
seconds, a 24.1% reduction. The fifth pair was contended. The result is below
the spike's 30% complete-slice target, so this hybrid is **not the chosen Gate 5
speed design**. Native ordinary AO preparation still takes about 8.5 seconds
of the candidate pass. There is no full PUI reindex or production speed claim.

Raw private reports are retained in ignored `rust-tools/benchmarks/`, with the
pair index at `pui_spike_pairs_20261115_194109.json`. The earlier bounded
fan-out version is at `pui_spike_pairs_20261115_193557.json`. The release
binary was built from a dirty local branch; no clean-commit acceptance run was
attempted because the spike failed its threshold. Isolated profiling Solr was
stopped throughout. No stage core was created or promoted.

The spike tools are `pui_resource_tree_reference.rb`,
`pui_ordinary_reference.rb`, `run_pui_resource_spike_pairs.py`, and the
`aspace-cli pui-tree-spike` command. The pair runner is deliberately pinned to
the existing isolated stack and saved reference. It is experimental tooling,
not an indexer replacement. A new PUI design must own ordinary AO fetch,
inheritance, publication/privacy, and output without sending full records back
through Ruby; it needs its own parity, fallback, Solr and performance gate.
