"""句読点を付けるモデル（日本語 BERT の文字単位のトークン分類。Mojicast が k2 の出力に使う ishiki-emo/mojicast-punct-onnx）を比べる。
k2 は句読点を出さないので、確定の字幕に句読点を付けるならこの工程が要る。
台本（data/stt_script.json）の文から句読点を抜いてモデルに通し、台本の句読点と位置が合うかを数える（、と。を別々に。文末の。は別に数える）。
しきい値と後処理（数字の後ろの句読点を消す）は Mojicast の punct.py と同じ。

usage: python3 punct_eval.py MODEL_DIR [--int8] [--k2 RESULT_KEY]
  onnxruntime と numpy の入った Python で動かす。MODEL_DIR は ishiki-emo/mojicast-punct-onnx を広げたフォルダ。
  `--k2 local/reazonspeech-k2-v2@1t+all` を付けると、results/stt_terms.json のその結果の文にも付けて、例を出す
"""

import json
import re
import sys
import time
from pathlib import Path

import numpy
import onnxruntime

HERE = Path(__file__).parent
NUM_FIX = re.compile(r"(\d)[、。](?=[年月日時分秒歳才人個円回本枚台匹冊話曲位点倍万億兆つ杯件番週%％])")


def load(folder: Path, int8: bool):
    vocab = {line.rstrip("\n"): i for i, line in enumerate((folder / "vocab.txt").open(encoding="utf-8"))}
    options = onnxruntime.SessionOptions()
    options.intra_op_num_threads = 4
    session = onnxruntime.InferenceSession(
        str(folder / ("punct_bert.int8.onnx" if int8 else "punct_bert.onnx")), options, providers=["CPUExecutionProvider"]
    )

    def punctuate(text: str) -> str:
        ids = numpy.array([[vocab["[CLS]"], *(vocab.get(c, vocab.get("[UNK]", 1)) for c in text), vocab["[SEP]"]]], dtype=numpy.int64)
        logits = session.run(None, {"input_ids": ids, "attention_mask": numpy.ones_like(ids)})[0][0]
        probs = 1 / (1 + numpy.exp(-logits))
        out = ""
        for j, c in enumerate(text):
            comma, period = probs[j + 1]
            out += c + ("。" if period > 0.1 else "、" if comma > 0.1 else "")
        return NUM_FIX.sub(r"\1", out)

    return punctuate


def marks(text: str) -> dict[int, str]:
    """句読点を抜いた文の何文字目の後ろに、どの句読点があるか（、と。のほかの記号は抜くだけ）"""
    found, n = {}, 0
    for c in text:
        if c in "、。！？!?":
            if n:
                found[n] = "。" if c != "、" else "、"
        else:
            n += 1
    return found


def strip(text: str) -> str:
    return re.sub(r"[、。！？!?\s]", "", text)


def main() -> None:
    args = sys.argv[1:]
    punctuate = load(Path(args[0]), "--int8" in args)
    script = json.loads((HERE / "data/stt_script.json").read_text())
    counts = {k: {"hit": 0, "want": 0, "got": 0} for k in ["、", "。", "文末"]}
    times, wrong = [], []
    for line in script:
        bare = strip(line["text"])
        started = time.monotonic()
        result = punctuate(bare)
        times.append((time.monotonic() - started) * 1000)
        want, got = marks(line["text"]), marks(result)
        end = len(bare)
        for kind, w, g in [
            ("、", {k for k, v in want.items() if v == "、"}, {k for k, v in got.items() if v == "、"}),
            ("。", {k for k, v in want.items() if v == "。" and k != end}, {k for k, v in got.items() if v == "。" and k != end}),
            ("文末", {end} & {k for k, v in want.items() if v == "。"}, {end} & {k for k, v in got.items() if v == "。"}),
        ]:
            counts[kind]["hit"] += len(w & g)
            counts[kind]["want"] += len(w)
            counts[kind]["got"] += len(g)
        if want != got:
            wrong.append({"want": line["text"], "got": result})
    times.sort()
    print(f"{'int8' if '--int8' in args else 'fp32'}: {len(script)} 文、1 文 {times[len(times) // 2]:.1f} ms（中央値）")
    for kind, c in counts.items():
        print(f"  {kind}: 正解 {c['want']}、付けた {c['got']}、合った {c['hit']}")
    print(f"  全部合った文: {len(script) - len(wrong)} / {len(script)}")
    for w in wrong[:12]:
        print(f"  台本 {w['want']}\n  BERT {w['got']}")
    if "--k2" in args:
        runs = json.loads((HERE / "results/stt_terms.json").read_text())[args[args.index("--k2") + 1]]["runs"]
        print("k2 の出力に付けた例:")
        for r in runs[:10]:
            print(f"  {punctuate(strip(r['heard']))}")


if __name__ == "__main__":
    main()
