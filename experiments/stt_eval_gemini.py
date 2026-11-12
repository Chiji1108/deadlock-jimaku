"""Google の Gemini 3.5 Transcribe の確定の文字起こしを、合成音声で測る（DESIGN.md 3 章）。
`crates/core/examples/stt_terms_eval.rs` と同じ台本・音声・数え方（用語を書けたか、文字誤り率）。
用語は、辞書の日本語の公式名を `custom_vocabulary` に渡す（アプリがほかのエンジンに渡すヒントと同じもの）。

usage: python3 stt_eval_gemini.py GLOSSARY.json [--no-vocabulary]
  GEMINI_API_KEY を使う。結果は results/stt_terms.json に足す
"""

import base64
import json
import os
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

HERE = Path(__file__).parent
URL = "https://generativelanguage.googleapis.com/v1beta/interactions"
MODEL = "gemini-3.5-transcribe"
USD_PER_MILLION = (2.0, 12.0)  # 音声の入力・文字の出力


def normalize(text: str) -> str:
    return "".join(c for c in text if c.isalnum())


def distance(a: str, b: str) -> int:
    row = list(range(len(b) + 1))
    for i, x in enumerate(a):
        previous, row[0] = row[0], i + 1
        for j, y in enumerate(b):
            previous, row[j + 1] = row[j + 1], min(previous + (x != y), row[j + 1] + 1, row[j] + 1)
    return row[len(b)]


def transcribe(wav: bytes, vocabulary: list[str]) -> dict:
    config = {"language_codes": ["ja-JP"]}
    if vocabulary:
        config["custom_vocabulary"] = vocabulary
    body = {
        "model": f"models/{MODEL}",
        "input": [{"type": "audio", "data": base64.b64encode(wav).decode(), "mime_type": "audio/wav"}],
        "generation_config": {"transcription_config": config},
    }
    request = urllib.request.Request(
        URL,
        data=json.dumps(body).encode(),
        headers={"x-goog-api-key": os.environ["GEMINI_API_KEY"], "Content-Type": "application/json"},
    )
    for attempt in range(5):
        started = time.monotonic()
        try:
            with urllib.request.urlopen(request, timeout=120) as res:
                answer = json.loads(res.read())
            text = "".join(c.get("text", "") for step in answer["steps"] for c in step.get("content", []))
            return {"text": text, "ms": round((time.monotonic() - started) * 1000), "usage": answer["usage"]}
        except urllib.error.HTTPError as e:
            detail = e.read().decode()
            print(f"  {e.code} {detail[:200]}", file=sys.stderr, flush=True)
            # 1 日の回数の上限は、待っても通らない（無料枠は 1 日 25 回）。そこまでの結果を残して終わる
            if "per day" in detail:
                raise Quota(detail) from e
            time.sleep(5 * (attempt + 1))
    return {"text": "", "ms": 0, "usage": {}, "failed": True}


class Quota(Exception):
    """1 日の回数の上限に届いた"""


def main() -> None:
    glossary = json.loads(Path(sys.argv[1]).read_text())
    names = [e["names"]["ja"] for e in glossary["entities"].values() if e["names"].get("ja")]
    vocabulary = [] if "--no-vocabulary" in sys.argv else sorted(set(names))
    script = json.loads((HERE / "data/stt_script.json").read_text())
    runs, hit, terms, edits, length, failed, usd = [], 0, 0, 0, 0, 0, 0.0
    stopped = False
    for voice in sorted(p for p in (HERE / "data/tts").iterdir() if p.is_dir()):
        for i, line in enumerate(script):
            try:
                got = transcribe((voice / f"{i:03}.wav").read_bytes(), vocabulary)
            except Quota:
                stopped = True
                break
            failed += bool(got.get("failed"))
            written = normalize(got["text"])
            missed = [
                term
                for term in line["terms"]
                if not any(normalize(s) in written for s in [term, *line.get("accept", {}).get(term, [])])
            ]
            terms += len(line["terms"])
            hit += len(line["terms"]) - len(missed)
            edits += distance(normalize(line["text"]), written)
            length += len(normalize(line["text"]))
            usage = got["usage"]
            usd += (usage.get("total_input_tokens", 0) * USD_PER_MILLION[0] + usage.get("total_output_tokens", 0) * USD_PER_MILLION[1]) / 1e6
            runs.append({"voice": voice.name, "line": i, "heard": got["text"], "missed": missed, "ms": got["ms"]})
            time.sleep(1.0)
        if stopped:
            break
    if not runs:
        sys.exit("今日はもう頼めません（1 日の回数の上限）")
    times = sorted(r["ms"] for r in runs if r["ms"])
    summary = {
        "model": MODEL,
        "vocabulary": len(vocabulary),
        "terms": terms,
        "termsWritten": hit,
        "characters": length,
        "characterErrors": edits,
        "failed": failed,
        "stoppedByQuota": stopped,
        "medianMs": times[len(times) // 2],
        "p90Ms": times[len(times) * 9 // 10],
        "usd": round(usd, 5),
    }
    print(summary)
    path = HERE / "results/stt_terms.json"
    results = json.loads(path.read_text())
    results["gemini35Transcribe" + ("" if vocabulary else "NoVocabulary")] = {"summary": summary, "runs": runs}
    path.write_text(json.dumps(results, ensure_ascii=False, indent=1))


if __name__ == "__main__":
    main()
