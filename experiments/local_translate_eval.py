# /// script
# requires-python = ">=3.12"
# dependencies = ["mlx-lm", "psutil", "transformers", "numba"]
# ///
"""PC の中で動く翻訳専門のモデル（PLaMo 3 Translate の MLX 版。Apple シリコンの GPU で動く）で、用語を決めた文を訳す（DESIGN.md 9.7）。
文と正解の用語は split_eval の translate と同じ（results/terms_cases.json）。用語はモデルの用語集の機能で渡す
（話した言葉 → ⟦英語名⟧）。訳文に用語の英語名が残っているか、1 発話（英語とロシア語）にかかる時間、メモリを見る。

usage: uv run local_translate_eval.py GLOSSARY.json REPO
  例: uv run local_translate_eval.py …/glossary.json pfnet/plamo-3-translate-0.8b-MLX-4bit
  GGUF 版は、llama-cpp-python の同梱の llama.cpp が古くて読み込めなかった（2026-11）
"""

import json
import re
import sys
import time
from pathlib import Path

import mlx.core as mx
import psutil
from mlx_lm import load, stream_generate

HERE = Path(__file__).parent
TARGETS = ["en", "ru"]


def prompt(text: str, terms: list[tuple[str, str]], target: str) -> str:
    glossary = "".join(f"<|plamo:index|>{spoken}<|plamo:item|>⟦{name}⟧" for spoken, name in terms)
    return (
        "<|plamo:tag|>dataset<|plamo:msg|>translation"
        + (f"<|plamo:tag|>glossary{glossary}" if terms else "")
        + f"<|plamo:tag|>input<|plamo:msg|>{text}"
        + f"<|plamo:tag|>output<|plamo:key|>lang<|plamo:val|>{target}<|plamo:msg|>"
    )


def kept(translation: str, name: str) -> bool:
    """英語名が、別の単語の一部としてではなく残っている"""
    return re.search(rf"(?<![0-9A-Za-z]){re.escape(name)}(?![0-9A-Za-z])", translation) is not None


STOPS = ["<|plamo:tag|>", "<|plamo:bos|>"]


def main() -> None:
    args = sys.argv[1:]
    entities = json.loads(Path(args[0]).read_text())["entities"]
    repo = args[1]
    t0 = time.monotonic()
    # モデルの組み立てとトークナイザは、モデルと一緒に配られている開発元のコードで動く
    model, tokenizer = load(repo, tokenizer_config={"trust_remote_code": True}, trust_remote_code=True)
    load_s = time.monotonic() - t0

    def translate(text: str) -> str:
        out = ""
        for chunk in stream_generate(model, tokenizer, text, max_tokens=256):
            out += chunk.text
            if any(stop in out for stop in STOPS):
                break
        return re.split("|".join(map(re.escape, STOPS)), out)[0].strip()

    runs, times = [], []
    for case in json.loads((HERE / "results/terms_cases.json").read_text()):
        segments = case["llm"][0]["answer"]["segments"]
        terms = [(s["text"], entities[s["ref"]]["names"]["en"]) for s in segments if s["ref"]]
        t0 = time.monotonic()
        translations = {}
        for target in TARGETS:
            translations[target] = translate(prompt(case["text"], terms, target))
        ms = round((time.monotonic() - t0) * 1000)
        times.append(ms)
        problems = {
            lang: [name for _, name in terms if not kept(t, name)] for lang, t in translations.items()
        }
        runs.append({
            "text": case["text"], "ms": ms, "ok": not any(problems.values()),
            "translations": translations, "missing": problems,
        })
    times.sort()
    summary = {
        "runs": len(runs),
        "ok": sum(r["ok"] for r in runs),
        "medianMs": times[len(times) // 2],
        "p90Ms": times[len(times) * 9 // 10],
        "loadSeconds": round(load_s, 1),
        "memoryMb": round(psutil.Process().memory_info().rss / 1e6),
        "gpuPeakMb": round(mx.get_peak_memory() / 1e6),
    }
    key = repo
    out = HERE / "results/local_translate_eval.json"
    report = json.loads(out.read_text()) if out.exists() else {}
    report[key] = {"summary": summary, "runs": runs}
    out.write_text(json.dumps(report, ensure_ascii=False, indent=1) + "\n")
    print(key, json.dumps(summary))


if __name__ == "__main__":
    main()
