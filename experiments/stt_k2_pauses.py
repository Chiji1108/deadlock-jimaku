"""補助プログラム jimaku-sherpa（crates/sherpa。ReazonSpeech k2 と、話の間の空白）に、録音を 1 本ずつ渡して書いた文を残す。
句読点の測定（tidy_eval.py の k2-pauses）の入力にする。

usage: python3 stt_k2_pauses.py JIMAKU_SHERPA MODEL_DIR HOTWORDS.txt
  soundfile の入った Python で動かす。MODEL_DIR は reazon-research/reazonspeech-k2-v2 を広げたフォルダ。
  ElevenLabs の 126 発話（data/tts）と本物の声（data/sample01、02）を書き、results/k2_pauses.json に残す
"""

import json
import struct
import subprocess
import sys
from pathlib import Path

import soundfile

HERE = Path(__file__).parent


def main() -> None:
    exe, model, hotwords = sys.argv[1:4]
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

    script = [line["text"] for line in json.loads((HERE / "data/stt_script.json").read_text())]
    voices = sorted(p.name for p in (HERE / "data/tts").iterdir() if p.is_dir())
    tts = [{"ref": script[i], "heard": ask(HERE / f"data/tts/{v}/{i:03}.wav")} for v in voices for i in range(len(script))]
    real = [{"ref": (HERE / f"data/{n}.txt").read_text().strip(), "heard": ask(HERE / f"data/{n}.wav")} for n in ["sample01", "sample02"]]
    (HERE / "results/k2_pauses.json").write_text(json.dumps({"tts": tts, "real": real}, ensure_ascii=False, indent=1) + "\n")


if __name__ == "__main__":
    main()
