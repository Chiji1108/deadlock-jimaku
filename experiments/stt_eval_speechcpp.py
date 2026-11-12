# /// script
# requires-python = ">=3.12"
# dependencies = ["httpx"]
# ///
"""PC の中で動く文字起こし（speech.cpp の Haas-ja など）を、stt_eval_openai.py と同じ指標（文字誤り率）で測る。
用語は `--keywords`（辞書の日本語の公式名。1 行に 1 つ）で渡す。

usage: uv run stt_eval_speechcpp.py SPEECH data/sample01.wav data/sample01.txt MODEL...
  SPEECH は speech.cpp の `speech` コマンドの場所。MODEL は `haas-ja-v1.0` や `haas-ja-v1.0:q4_k`
  時間は speech が書く認識の時間（モデルの読み込みは入らない）。GPU と CPU だけ（4 スレッド）で測る
"""

import json
import re
import subprocess
import sys
from pathlib import Path

from stt_eval_xai_qwen import cer, load_keywords


def transcribe(speech: str, model: str, wav: Path, options: list[str]) -> dict:
    r = subprocess.run([speech, "asr", model, *options, str(wav)], capture_output=True, text=True, check=True)
    seconds = re.search(r"of audio in ([\d.]+) s", r.stderr)
    return {"text": r.stdout.strip(), "latency_s": float(seconds.group(1)) if seconds else None}


def main() -> None:
    speech, wav, ref = sys.argv[1], Path(sys.argv[2]), Path(sys.argv[3]).read_text().strip()
    keywords = "\n".join(load_keywords())
    conditions = {
        "gpu": [],
        "gpu/keywords": ["--keywords", keywords],
        "cpu-4threads": ["--device", "cpu", "--threads", "4"],
    }
    results: dict[str, list] = {}
    for model in sys.argv[4:]:
        for name, options in conditions.items():
            runs = results.setdefault(f"{model}/{name}", [])
            for _ in range(3):
                try:
                    r = transcribe(speech, model, wav, options)
                    r["cer"] = round(cer(ref, r["text"]), 4)
                except subprocess.CalledProcessError as e:
                    r = {"text": "", "error": e.stderr[-300:]}
                runs.append(r)
    out = Path(__file__).parent / "results" / f"{wav.stem}.speechcpp.json"
    out.write_text(json.dumps(results, ensure_ascii=False, indent=2) + "\n")
    for name, runs in results.items():
        print(f"## {name}")
        for r in runs:
            print(f"  {r.get('error') or f'CER {r["cer"]:.1%}  {r["latency_s"]}s'}\n  {r['text']}")


if __name__ == "__main__":
    main()
