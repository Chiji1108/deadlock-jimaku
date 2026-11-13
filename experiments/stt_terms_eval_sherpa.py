"""SenseVoice（sherpa-onnx で動かす）を、合成音声で比べる（DESIGN.md 3 章）。数え方は stt_terms_eval_local.py と同じ。

usage: python3 stt_terms_eval_sherpa.py NAME MODEL_DIR [--int8] [--threads N]
  sherpa-onnx の入った Python で動かす（`pip install sherpa-onnx soundfile`）。MODEL_DIR は sherpa-onnx の配布物を広げたフォルダ
"""

import sys
import time
from pathlib import Path

import sherpa_onnx
import soundfile

from stt_terms_eval_local import score


def main() -> None:
    args = sys.argv[1:]
    name, folder = args[0], Path(args[1])
    threads = int(args[args.index("--threads") + 1]) if "--threads" in args else 4
    recognizer = sherpa_onnx.OfflineRecognizer.from_sense_voice(
        model=str(folder / ("model.int8.onnx" if "--int8" in args else "model.onnx")),
        tokens=str(folder / "tokens.txt"),
        num_threads=threads,
        language="ja",
        use_itn=True,
    )

    def transcribe(wav: Path) -> dict:
        samples, rate = soundfile.read(wav, dtype="float32")
        started = time.monotonic()
        stream = recognizer.create_stream()
        stream.accept_waveform(rate, samples)
        recognizer.decode_stream(stream)
        return {"text": stream.result.text, "ms": round((time.monotonic() - started) * 1000)}

    score(name, "none", transcribe)


if __name__ == "__main__":
    main()
