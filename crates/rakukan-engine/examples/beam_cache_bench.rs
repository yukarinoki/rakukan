//! Evaluate model-only beam search on AJIMEE-Bench without downloading data.
//! Usage: beam_cache_bench MODEL.gguf tokenizer.json evaluation_items.json
use rakukan_engine::kanji::{build_jinen_prompt, llamacpp::LlamaCppModel};
use std::io::{BufWriter, Write};
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 4 {
        return Err(
            "usage: beam_cache_bench MODEL.gguf tokenizer.json evaluation_items.json".into(),
        );
    }
    let mut model = LlamaCppModel::from_file(&args[1], &args[2])?;
    model.set_n_threads(4);
    let cases: Vec<serde_json::Value> = serde_json::from_str(&std::fs::read_to_string(&args[3])?)?;
    let eos = Some(model.eos_token_id().0);
    let mut output = BufWriter::new(std::io::stdout().lock());
    for case in cases {
        let reading = case["input"].as_str().ok_or("missing input")?;
        let context = case["context_text"].as_str().unwrap_or("");
        let tokens = model.tokenize(&build_jinen_prompt(reading, context))?;
        let budget = (reading.chars().count() * 2 + 8).clamp(15, 256);
        for width in [1, 3, 6] {
            let start = Instant::now();
            let candidates = model.generate_beam_search(&tokens, budget, eos, width)?;
            let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
            let texts: Vec<_> = candidates
                .iter()
                .map(|c| model.decode(&c.0, true))
                .collect::<Result<_, _>>()?;
            let ids: Vec<Vec<_>> = candidates
                .iter()
                .map(|c| c.0.iter().map(|t| t.0).collect())
                .collect();
            let row = serde_json::json!({
                "case":case["index"], "reading":reading, "context":context,
                "expected_output":case["expected_output"], "width":width,
                "budget":budget, "elapsed_ms":elapsed_ms, "candidates":texts,
                "tokens":ids, "scores":candidates.iter().map(|c| c.1).collect::<Vec<_>>()
            });
            writeln!(output, "{row}")?;
            output.flush()?;
        }
    }
    Ok(())
}
