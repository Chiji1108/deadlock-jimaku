"""補助プログラム jimaku-sherpa（ReazonSpeech k2 と、話の間の空白）に、数字の台本（data/tidy_script.json）の録音を渡して書いた文を残す。
数字の測定（tidy_eval.py の --k2-tidy）の入力にする。

usage: python3 stt_k2_tidy.py JIMAKU_SHERPA MODEL_DIR HOTWORDS.txt AUDIO_DIR OUT.json
  soundfile の入った Python で動かす。AUDIO_DIR は 声の名前/000.wav … のフォルダ（tts_generate.py の data/tts_tidy など）
"""

import json
import struct
import subprocess
import sys
from pathlib import Path

import soundfile

HERE = Path(__file__).parent


def main() -> None:
    exe, model, hotwords, audio, out = sys.argv[1:6]
    files = [f"{model}/{name}-epoch-99-avg-1.onnx" for name in ("encoder", "decoder", "joiner")]
    helper = subprocess.Popen([exe, *files, f"{model}/tokens.txt", hotwords], stdin=subprocess.PIPE, stdout=subprocess.PIPE)
    assert json.loads(helper.stdout.readline()) == {"ready": True}

    def ask(path: Path) -> str:
        samples, _ = soundfile.read(path, dtype="float32")
        if samples.ndim > 1:
            samples = samples.mean(axis=1)
        helper.stdin.write(struct.pack("<I", samples.size) + samples.astype("<f4").tobytes())
        helper.stdin.flush()
        return json.loads(helper.stdout.readline())["text"]

    count = len(json.loads((HERE / "data/tidy_script.json").read_text()))
    voices = sorted(p for p in Path(audio).iterdir() if p.is_dir())
    rows = [{"line": i, "voice": v.name, "heard": ask(v / f"{i:03}.wav")} for v in voices for i in range(count)]
    Path(out).write_text(json.dumps(rows, ensure_ascii=False, indent=1) + "\n")


if __name__ == "__main__":
    main()
