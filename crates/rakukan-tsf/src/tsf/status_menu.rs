//! Menu diagnostics: cached readiness/RPC observations, bounded existing log read.
//! Never starts the engine, sends RPC, waits for its lock, or performs inference.
use crate::engine::state::{RAKUKAN_ENGINE, last_observed_readiness};
use std::io::{Read, Seek, SeekFrom};

pub fn lines() -> Vec<String> {
    let engine = match RAKUKAN_ENGINE.try_lock() {
        Ok(guard) => match guard.0.as_ref() {
            Some(engine) => {
                let (failed, age) = engine.last_response_health();
                if failed {
                    "エンジン：直近の通信／処理でエラー".into()
                } else if age.as_secs() >= 30 {
                    format!("エンジン：未確認（最終応答 {} 秒前）", age.as_secs())
                } else {
                    format!("エンジン：応答 OK（{} 秒前）", age.as_secs())
                }
            }
            None => "エンジン：未接続／初期化中".into(),
        },
        Err(_) => "エンジン：処理中（状態取得を省略）".into(),
    };
    let (dict, model) = last_observed_readiness();
    let mut result = vec![
        engine,
        format!(
            "辞書（最終確認）：{}",
            if dict {
                "読み込み済み"
            } else {
                "未準備"
            }
        ),
        format!(
            "モデル（最終確認）：{}",
            if model { "準備完了" } else { "未準備" }
        ),
    ];
    let speed = log_tail().ok().and_then(|text| last_conversion(&text));
    result.push(match speed.as_ref() {
        Some((ms, chars, _)) => format!(
            "直近のモデル生成：{} ms・{}文字（{}）",
            ms,
            chars,
            speed_label(*ms)
        ),
        None => "モデル生成速度：未計測／ログに記録なし".into(),
    });
    if let Some((_, _, timestamp)) = speed {
        result.push(format!("生成の記録時刻：{}", timestamp));
    }
    result
}

fn log_tail() -> std::io::Result<String> {
    let directory = std::env::var_os("LOCALAPPDATA").ok_or(std::io::ErrorKind::NotFound)?;
    let mut file = std::fs::File::open(
        std::path::PathBuf::from(directory)
            .join("rakukan")
            .join("rakukan-engine-dll.log"),
    )?;
    let len = file.metadata()?.len();
    file.seek(SeekFrom::Start(len.saturating_sub(64 * 1024)))?;
    let mut bytes = Vec::new();
    file.take(64 * 1024).read_to_end(&mut bytes)?;
    let text = String::from_utf8_lossy(&bytes);
    // The first line may be truncated; the last line may still be being written.
    let start = if len > 64 * 1024 {
        text.find('\n').map_or(text.len(), |p| p + 1)
    } else {
        0
    };
    let end = text.rfind('\n').map_or(start, |p| p + 1);
    Ok(text[start..end.max(start)].to_string())
}

fn number(line: &str, key: &str) -> Option<u64> {
    line.split_whitespace()
        .find_map(|part| part.strip_prefix(key)?.parse().ok())
}

fn last_conversion(text: &str) -> Option<(u64, u64, String)> {
    text.lines().rev().find_map(|line| {
        if !line.contains("beam conversion done") {
            return None;
        }
        Some((
            number(line, "elapsed_ms=")?,
            number(line, "reading_chars=")?,
            line.split_whitespace().next()?.to_string(),
        ))
    })
}

fn speed_label(ms: u64) -> &'static str {
    match ms {
        0..=200 => "速い目安",
        201..=999 => "待ち時間あり",
        _ => "遅い目安",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn speed_uses_latest_complete_success_without_exposing_input() {
        let text = "2026-09-09T10:00:00Z INFO beam conversion done reading_chars=8 elapsed_ms=1500\n2026-09-09T10:01:00Z INFO beam conversion done reading_chars=3 elapsed_ms=67\nother event elapsed_ms=1\n";
        assert_eq!(
            last_conversion(text),
            Some((67, 3, "2026-09-09T10:01:00Z".into()))
        );
        assert_eq!(last_conversion("beam conversion done elapsed_ms=bad"), None);
        assert_eq!(speed_label(200), "速い目安");
        assert_eq!(speed_label(201), "待ち時間あり");
        assert_eq!(speed_label(1000), "遅い目安");
    }
}
