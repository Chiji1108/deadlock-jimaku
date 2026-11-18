"""ReazonSpeech k2 v2（sherpa-onnx で動かす。Mojicast の既定）を、合成音声で比べる（DESIGN.md 3 章）。数え方は stt_terms_eval_local.py と同じ。
前後に 0.9 秒の無音を足す（ReazonSpeech の公式の処理と Mojicast と同じ。足さないと、最後の句より前が抜ける）。
用語は、辞書の日本語の公式名をホットワード（modified_beam_search のコンテキストバイアス）として渡した場合も測る。
Mojicast と同じく、ホットワードを渡すときだけ modified_beam_search にする（greedy_search はホットワードを使えない）。

usage: python3 stt_terms_eval_k2.py MODEL_DIR GLOSSARY.json [--int8] [--threads N] [--hotwords none,heroes,all] [--score 2.0]
  sherpa-onnx の入った Python で動かす（`pip install sherpa-onnx soundfile`）。
  MODEL_DIR は reazon-research/reazonspeech-k2-v2 を広げたフォルダ
"""

import json
import sys
import tempfile
import time
from pathlib import Path

import numpy
import sherpa_onnx
import soundfile

from stt_terms_eval_local import score


def main() -> None:
    args = sys.argv[1:]
    folder, glossary_path = Path(args[0]), Path(args[1])
    threads = int(args[args.index("--threads") + 1]) if "--threads" in args else 1
    which = args[args.index("--hotwords") + 1].split(",") if "--hotwords" in args else ["none", "all"]
    boost = float(args[args.index("--score") + 1]) if "--score" in args else 2.0
    suffix = ".int8.onnx" if "--int8" in args else ".onnx"
    glossary = json.loads(glossary_path.read_text())
    names = lambda kinds: sorted({e["names"]["ja"] for e in glossary["entities"].values() if e["kind"] in kinds and e["names"].get("ja")})
    lists = {"none": None, "heroes": names({"hero"}), "all": names({"hero", "item", "ability"})}
    for kind in which:
        hotwords = ""
        if lists[kind]:
            file = tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False, encoding="utf-8")
            file.write("\n".join(lists[kind]) + "\n")
            file.close()
            hotwords = file.name
        recognizer = sherpa_onnx.OfflineRecognizer.from_transducer(
            encoder=str(folder / f"encoder-epoch-99-avg-1{suffix}"),
            decoder=str(folder / f"decoder-epoch-99-avg-1{suffix}"),
            joiner=str(folder / f"joiner-epoch-99-avg-1{suffix}"),
            tokens=str(folder / "tokens.txt"),
            num_threads=threads,
            decoding_method="modified_beam_search" if hotwords else "greedy_search",
            hotwords_file=hotwords,
            hotwords_score=boost,
            modeling_unit="cjkchar",
        )

        def transcribe(wav: Path) -> dict:
            samples, rate = soundfile.read(wav, dtype="float32")
            samples = numpy.pad(samples, int(0.9 * rate))
            started = time.monotonic()
            stream = recognizer.create_stream()
            stream.accept_waveform(rate, samples)
            recognizer.decode_stream(stream)
            return {"text": stream.result.text, "ms": round((time.monotonic() - started) * 1000)}

        name = "reazonspeech-k2-v2" + ("-int8" if "--int8" in args else "") + f"@{threads}t"
        score(name + ("" if kind == "none" or boost == 2.0 else f"x{boost:g}"), kind, transcribe)


if __name__ == "__main__":
    main()
