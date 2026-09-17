# receiver-gate census

method: gate-audit-v1 (index-side census of raw_edges receiver calls, classified by cort::graph's own gate primitives)

reading: zero_candidates counts calls whose method name the project never declares — std, dependencies, iterator adapters; it is the static-analysis frontier, not a recall leak. binding_refused holds the type-directed-dispatch candidates; read its examples.

Refusal examples per class are in the JSON report under `refused_classes`; each carries file:line and is checkable by hand. A number in this table is quotable only with its row's commit and machine.

| venue | venue_head | index_head | heads_agree | population | attached | refused | zero_candidates | multiple_candidates | one_ownerless | binding_refused | no_receiver_shape | machine |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| . | b1730bd6 | b1730bd63e6f41e5574474357d3eb10a9778d896 | yes | 10030 | 34 | 9996 | 9643 | 186 | 146 | 21 | 0 | 2eb02d46ec5c4319/etc-machine-id |

