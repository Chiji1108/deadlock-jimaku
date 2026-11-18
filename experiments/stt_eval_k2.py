"""ReazonSpeech k2 v2（sherpa-onnx）を、録音で stt_eval_speechcpp.py と同じ指標（文字誤り率）で測る。
録音をまるごと 1 回で渡す（前後に 0.9 秒の無音を足す。stt_terms_eval_k2.py と同じ）。
用語は、辞書の日本語の公式名（ヒーロー・アイテム・スキル）をホットワードで渡した場合も測る。

usage: python3 stt_eval_k2.py MODEL_DIR GLOSSARY.json data/sample01.wav data/sample01.txt
  sherpa-onnx の入った Python で動かす（`pip install sherpa-onnx soundfile numpy`）。結果は results/<録音>.k2.json
"""

import json
import sys
import tempfile
import time
from pathlib import Path

import numpy
import sherpa_onnx
import soundfile

from stt_eval_xai_qwen import cer


def main() -> None:
    folder, glossary_path, wav, ref = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3]), Path(sys.argv[4]).read_text().strip()
    glossary = json.loads(glossary_path.read_text())
    names = sorted({e["names"]["ja"] for e in glossary["entities"].values() if e["kind"] in {"hero", "item", "ability"} and e["names"].get("ja")})
    hotwords = tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False, encoding="utf-8")
    hotwords.write("\n".join(names) + "\n")
    hotwords.close()
    samples, rate = soundfile.read(wav, dtype="float32")
    samples = numpy.pad(samples, int(0.9 * rate))
    results = {}
    for name, words in {"none": "", "hotwords": hotwords.name}.items():
        recognizer = sherpa_onnx.OfflineRecognizer.from_transducer(
            encoder=str(folder / "encoder-epoch-99-avg-1.onnx"),
            decoder=str(folder / "decoder-epoch-99-avg-1.onnx"),
            joiner=str(folder / "joiner-epoch-99-avg-1.onnx"),
            tokens=str(folder / "tokens.txt"),
            num_threads=1,
            decoding_method="modified_beam_search" if words else "greedy_search",
            hotwords_file=words,
            hotwords_score=2.0,
            modeling_unit="cjkchar",
        )
        started = time.monotonic()
        stream = recognizer.create_stream()
        stream.accept_waveform(rate, samples)
        recognizer.decode_stream(stream)
        text = stream.result.text
        results[f"reazonspeech-k2-v2/{name}"] = {"text": text, "cer": round(cer(ref, text), 4), "latency_s": round(time.monotonic() - started, 2)}
        print(name, results[f"reazonspeech-k2-v2/{name}"], flush=True)
    out = Path(__file__).parent / f"results/{wav.stem}.k2.json"
    out.write_text(json.dumps(results, ensure_ascii=False, indent=1) + "\n")


if __name__ == "__main__":
    main()
