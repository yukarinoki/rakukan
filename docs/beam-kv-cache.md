# Beam search KV-cache reuse

Beam search now prefills the prompt once and evaluates one pending token per
active beam on subsequent steps. Previously, every step cleared the cache and
evaluated every active beam's entire prompt and generated prefix again.

Each beam retains its parent sequence ID through candidate scoring and sorting.
Two disjoint banks of sequence IDs alternate between steps: destinations are
cleared, all surviving children inherit their parents' KV, and only then are
the source sequences removed. This handles both sibling forks and reordered
beams without overwriting a parent another child still needs. A separate
temporary sequence holds the initial prefill.

The context reserves capacity for both banks; the decode batch only needs room
for the prompt or one token per beam. Candidate scoring, expansion, pruning,
early stopping, the 15-second generation timeout, and EOS-only results are
preserved. Empty width/budget returns no candidates, while an empty prompt or
overflowing context dimensions returns an error.

## Numerical behavior and evaluation

Incremental and full-prefix evaluations may produce different logits. Exact
candidate equivalence to full recomputation is not a requirement: close scores
can reorder beams and alter pruning. In diagnostic checks, differences also
occurred without any KV copying, remained with F32 KV, and disappeared when
both evaluations used one-token microbatches. This does not establish the
historical GPT-2 cache-sharing issue mentioned by the old implementation.

The pre-integration comparison used:

- Baseline: commit `6e0daa77f36ef11a774aa2b9edab85ed9bb1a88c`.
- Model: `togatogah/jinen-v1-xsmall.gguf`, `jinen-v1-xsmall-Q5_K_M.gguf`.
- Runtime: llama-cpp-2 / llama-cpp-sys-2 0.1.143.
- Hardware: Intel Core Ultra 7 155U, CPU only, 4 threads.
- Dataset: [AJIMEE-Bench](https://github.com/azooKey/AJIMEE-Bench/tree/401666cd56d1a570c2021798b64b6da4396bfd45),
  `JWTD_v2/v1/evaluation_items.json`, all 200 inputs, including 100 with left context.
- Dataset SHA-256: `e9eb668fd6aa14b1e26436f429b5550108af0a1dfd443b8cea0bcb3ab3028fca`.
- Dataset license: CC-BY-SA 3.0; see the upstream README for original data attribution.

Inputs were not split. Generation budget was
`min(256, max(15, reading_character_count * 2 + 8))`.
Width 1 used beam search on both sides, not the separate greedy API.
Each condition had one timed run per implementation, alternating execution
order by input. Timing includes context creation but excludes model loading,
tokenization, UI work, and a separate per-step logit verification pass.

| Width | Median ms, replay → cached | p95 ms, replay → cached | Acc@1, both |
|---|---:|---:|---:|
| 1 | 172.3 → 64.0 | 1053.5 → 174.7 | 134/200 |
| 3 | 474.7 → 131.1 | 3194.2 → 379.9 | 135/200 |
| 6 | 939.3 → 243.0 | 6552.0 → 701.3 | 135/200 |

Ranked candidate lists differed in 10/600 conditions. The top candidate differed
for one input (`index=1236`) at all three widths; width 6 included the baseline's
15-second timeout and empty result. Cached inference completed every condition.
Acc@1 is exact membership in the dataset's accepted outputs, without text
normalization. This corpus is evidence for this model/runtime/CPU combination,
not a guarantee for other models or GPU backends.

## Reproducing checks

The normal engine unit tests need no model:

```text
cargo test -p rakukan-engine --lib
```

For the ignored model regression test, set `RAKUKAN_TEST_MODEL` to the local
jinen-v1-xsmall Q5_K_M GGUF and `RAKUKAN_TEST_TOKENIZER` to its tokenizer.json:

```text
cargo test -p rakukan-engine --release --lib beam_cache_jinen_regression -- --ignored
```

This checks common conversions at widths 1/3/6, sorted finite scores, EOS-only
completion, budget exhaustion, explicit EOS, invalid dimensions, and repeated
requests after intervening conversions. It does not download the model.

To evaluate the production API with a locally downloaded AJIMEE-Bench file:

```text
cargo run -p rakukan-engine --release --example beam_cache_bench -- MODEL.gguf tokenizer.json evaluation_items.json > results.jsonl
```

The example emits candidate texts, token IDs, scores, and model-only timings as
JSONL for each input at widths 1/3/6 using 4 CPU threads. Compare token IDs and
scores for implementation regressions; use accepted outputs to evaluate
accuracy rather than requiring identical outputs from full recomputation.

After integration, the production API was run on all 600 conditions. Candidate
texts, token IDs, and scores exactly matched the previously evaluated cached
implementation, including after reducing decode-batch capacity to the prompt
length or beam width. The engine unit tests (178 passed) and the explicit
model regression test also passed on this CPU setup.
