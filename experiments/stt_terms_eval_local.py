"""PC の中で動く文字起こし（speech.cpp か whisper.cpp が対応するモデル）を、合成音声で比べる（DESIGN.md 3 章）。
台本・音声・数え方は `crates/core/examples/stt_terms_eval.rs` と同じ（用語を書けたか、文字誤り率）。
モデルごとに `speech serve`（MODEL が `.bin` のファイルなら whisper.cpp の `whisper-server`）を立てて、
OpenAI と同じ形の口（/v1/audio/transcriptions）に 1 発話ずつ送る。
用語を渡せるモデルには、辞書の日本語の公式名を `prompt` で渡した場合も測る。

usage: python3 stt_terms_eval_local.py SPEECH GLOSSARY.json MODEL... [--prompts none,all,heroes]
  SPEECH は speech.cpp の `speech` コマンドの場所。MODEL は `speech models` の名前（qwen3-asr-1.7b など）か、
  whisper.cpp の ggml のファイル（ggml-large-v3-turbo.bin など。`whisper-server` は PATH から探す）。
  モデルは `SPEECH_MODEL_DIR`（なければ speech.cpp の既定の場所）に取る。結果は results/stt_terms.json に足す
"""

import json
import subprocess
import sys
import time
import urllib.error
import urllib.request
import uuid
from pathlib import Path

from stt_eval_gemini import distance, normalize

HERE = Path(__file__).parent
PORT = 8770
URL = f"http://127.0.0.1:{PORT}"


def post(wav: bytes, prompt: str | None) -> dict:
    boundary = uuid.uuid4().hex
    fields = {"language": "ja", "response_format": "json"}
    if prompt:
        fields["prompt"] = prompt
    body = b""
    for name, value in fields.items():
        body += f'--{boundary}\r\nContent-Disposition: form-data; name="{name}"\r\n\r\n{value}\r\n'.encode()
    body += (
        f'--{boundary}\r\nContent-Disposition: form-data; name="file"; filename="a.wav"\r\n'
        "Content-Type: audio/wav\r\n\r\n"
    ).encode() + wav + f"\r\n--{boundary}--\r\n".encode()
    request = urllib.request.Request(
        f"{URL}/v1/audio/transcriptions",
        data=body,
        headers={"Content-Type": f"multipart/form-data; boundary={boundary}"},
    )
    started = time.monotonic()
    try:
        with urllib.request.urlopen(request, timeout=120) as res:
            text = json.loads(res.read()).get("text", "")
        return {"text": text, "ms": round((time.monotonic() - started) * 1000)}
    except urllib.error.HTTPError as e:
        return {"text": "", "ms": 0, "error": e.read().decode()[:200]}


def serve(speech: str, model: str) -> subprocess.Popen:
    command = (
        ["whisper-server", "-m", model, "-l", "ja", "--port", str(PORT), "--inference-path", "/v1/audio/transcriptions"]
        if model.endswith(".bin")
        else [speech, "serve", model, "--port", str(PORT)]
    )
    process = subprocess.Popen(command, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    for _ in range(900):
        if process.poll() is not None:
            sys.exit(f"{model}: did not start")
        try:
            urllib.request.urlopen(f"{URL}/health", timeout=2)
            return process
        except OSError:
            time.sleep(2)
    process.kill()
    sys.exit(f"{model}: did not start")


def main() -> None:
    args = sys.argv[1:]
    which = args[args.index("--prompts") + 1].split(",") if "--prompts" in args else ["none", "all"]
    speech, glossary_path, *models = [a for a in args if not a.startswith("--") and a not in (",".join(which),)]
    glossary = json.loads(Path(glossary_path).read_text())
    names = lambda kinds: sorted({e["names"]["ja"] for e in glossary["entities"].values() if e["kind"] in kinds and e["names"].get("ja")})
    prompts = {
        "none": None,
        "all": "、".join(names({"hero", "item", "ability"})),
        "heroes": "、".join(names({"hero"})),
    }
    script = json.loads((HERE / "data/stt_script.json").read_text())
    voices = sorted(p for p in (HERE / "data/tts").iterdir() if p.is_dir())
    for model in models:
        process = serve(speech, model)
        try:
            for kind in which:
                runs, hit, terms, edits, length, failed = [], 0, 0, 0, 0, 0
                for voice in voices:
                    for i, line in enumerate(script):
                        got = post((voice / f"{i:03}.wav").read_bytes(), prompts[kind])
                        failed += "error" in got
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
                        runs.append({"voice": voice.name, "line": i, "heard": got["text"], "missed": missed, "ms": got["ms"], **({"error": got["error"]} if "error" in got else {})})
                times = sorted(r["ms"] for r in runs if r["ms"]) or [0]
                summary = {
                    "model": Path(model).stem if model.endswith(".bin") else model,
                    "prompt": kind,
                    "terms": terms,
                    "termsWritten": hit,
                    "characters": length,
                    "characterErrors": edits,
                    "failed": failed,
                    "medianMs": times[len(times) // 2],
                    "p90Ms": times[len(times) * 9 // 10],
                }
                key = f"local/{summary['model']}" + ("" if kind == "none" else f"+{kind}")
                print(f"{key}: {json.dumps(summary, ensure_ascii=False)}", flush=True)
                path = HERE / "results/stt_terms.json"
                results = json.loads(path.read_text())
                results[key] = {"summary": summary, "runs": runs}
                path.write_text(json.dumps(results, ensure_ascii=False, indent=1))
        finally:
            process.terminate()
            process.wait()


if __name__ == "__main__":
    main()
