# One-resource PUI indexing spike (bounded close-out)

This local experiment used `/repositories/4/resources/6` in the disposable
profiling stack. Ruby prepared ordinary archival-object PUI documents while a
separate Rust CLI fetched and built ordered-record and tree auxiliary documents
through the authoritative API. The two independent jobs ran concurrently.
The Rust result crossed back to Ruby as auxiliary documents only; ordinary
full-record JSON did not cross the boundary. No production PUI flag changed.

Five alternating read-only pairs each matched **2,843 pre-Solr documents**
type-strictly, including embedded JSON, plus **1,000 delete candidates**. The
native median was **13.380 s**; the parallel hybrid median was **8.875 s**, a
**33.7% reduction** for fetch and preparation. The sequential version reached
only 24.1%; concurrency is essential to the result. Raw pair reports are in
ignored `rust-tools/benchmarks/pui_spike_pairs_20261115_195316.json`. The
timed release binary SHA-256 was
`52d01ee4d4c61114cdb02a2b8111966c458bc9b325d9e600d36cac73c33afedf`.
The timed pairs exclude Solr writes, commits and report serialization.

One saved native/candidate pair was written to two fresh isolated Solr cores.
Both contained **2,843 stored documents with exact parity**. The report is
`rust-tools/benchmarks/pui_spike_solr_20261115_195954_2912879_06df47bc.json`.
Its individual write/commit times are descriptive; core creation order and
warm-up differ, so they are not a paired full-index speed benchmark. The
profiling Solr container was stopped after the check at about 4.5/6 GiB
memory use. Existing cores and both new stage cores were retained.

A forced Rust process failure produced a native auxiliary fallback with exact
pre-Solr parity for all 2,843 documents and 1,000 delete candidates. The
private report is `pui_full_reference_20261115_2008_fallback.json`. This tests
one failure path before Solr writes. The tree used here had 1,210 published,
unsuppressed ordinary AO documents; no unpublished child was available to
exercise suppression. A local scan found no nonempty embedded ancestors,
resolved top-container internal notes, resolved container-profile notes, or
`_archivalist_` markers in those documents. Broader privacy and plugin
coverage remains open.

**Decision:** the 30% read-only speed kill condition did not fire, so retain
the parallel design as a Gate 5 candidate. This is not a Gate 5 pass or a
production speed claim. A complete integrated pass with Solr writes,
publication edge cases, robust fallback, and larger trees is still required.
The spike ended within its four-active-hour limit. Gate 4 remains paused.

The tools are `pui_resource_tree_reference.rb`,
`pui_ordinary_reference.rb`, `run_pui_resource_spike_pairs.py`,
`run_pui_spike_solr_parity.py`, and `aspace-cli pui-tree-spike`. The pair runner
is pinned to the existing isolated stack and saved reference. These are
experimental tools, not a replacement indexer.
