"""FuguMT（日本語 → 英語だけの翻訳専門のモデル。Mojicast の英訳。CTranslate2 に変換した ishiki-emo/mojicast-fugumt-ja-en-ct2）で、
用語を決めた文を訳す（DESIGN.md 9.7）。文と正解の用語は local_translate_eval.py と同じ（results/terms_cases.json）。
FuguMT には用語集の機能がないので、渡し方を 3 つ比べる:
  plain   用語を英語名に置き換えた文（「InfernusのNapalmが痛すぎる。」。Chrome の翻訳に渡す形）
  bracket 英語名を ⟦ ⟧ で囲んだ文（mt.rs が翻訳専門のモデルに渡す形）
  raw     置き換えない元の文（用語の手当てなし）
デコードの設定は Mojicast と同じ（beam 8、repetition_penalty 1.2、no_repeat_ngram_size 3）。

usage: python3 fugumt_eval.py GLOSSARY.json MODEL_DIR [--threads N] [--fp32]
  ctranslate2 と sentencepiece の入った Python で動かす。結果は results/local_translate_eval.json に足す（英語だけ）
"""

import json
import re
import sys
import time
from pathlib import Path

import ctranslate2
import psutil
import sentencepiece

HERE = Path(__file__).parent


def kept(translation: str, name: str) -> bool:
    """英語名が、別の単語の一部としてではなく残っている（local_translate_eval.py と同じ）"""
    return re.search(rf"(?<![0-9A-Za-z]){re.escape(name)}(?![0-9A-Za-z])", translation) is not None


def main() -> None:
    args = sys.argv[1:]
    entities = json.loads(Path(args[0]).read_text())["entities"]
    folder = Path(args[1])
    threads = int(args[args.index("--threads") + 1]) if "--threads" in args else 4
    compute = "float32" if "--fp32" in args else "int8"
    t0 = time.monotonic()
    source = sentencepiece.SentencePieceProcessor(model_file=str(folder / "source.spm"))
    target = sentencepiece.SentencePieceProcessor(model_file=str(folder / "target.spm"))
    model = ctranslate2.Translator(str(folder), device="cpu", compute_type=compute, inter_threads=1, intra_threads=threads)
    load_s = time.monotonic() - t0

    def translate(text: str) -> str:
        tokens = source.encode(text, out_type=str)[:511] + ["</s>"]
        result = model.translate_batch([tokens], beam_size=8, repetition_penalty=1.2, no_repeat_ngram_size=3, max_decoding_length=96)
        return target.decode([t for t in result[0].hypotheses[0] if t not in ("</s>", "<pad>", "<unk>")]).strip()

    cases = json.loads((HERE / "results/terms_cases.json").read_text())
    for form in ["plain", "bracket", "raw"]:
        runs, times = [], []
        for case in cases:
            segments = case["llm"][0]["answer"]["segments"]
            terms = [entities[s["ref"]]["names"]["en"] for s in segments if s["ref"]]
            sent = "".join(
                s["text"] if not s["ref"] or form == "raw"
                else f"⟦{entities[s['ref']]['names']['en']}⟧" if form == "bracket"
                else entities[s["ref"]]["names"]["en"]
                for s in segments
            )
            started = time.monotonic()
            en = translate(sent)
            ms = round((time.monotonic() - started) * 1000)
            times.append(ms)
            missing = [name for name in terms if not kept(en, name)]
            runs.append({"text": case["text"], "sent": sent, "ms": ms, "ok": not missing, "translations": {"en": en}, "missing": {"en": missing}})
        times.sort()
        summary = {
            "runs": len(runs),
            "ok": sum(r["ok"] for r in runs),
            "medianMs": times[len(times) // 2],
            "p90Ms": times[len(times) * 9 // 10],
            "loadSeconds": round(load_s, 1),
            "memoryMb": round(psutil.Process().memory_info().rss / 1e6),
            "languages": ["en"],
        }
        key = f"fugumt-ct2-{compute}-{threads}threads+{form}"
        out = HERE / "results/local_translate_eval.json"
        report = json.loads(out.read_text())
        report[key] = {"summary": summary, "runs": runs}
        out.write_text(json.dumps(report, ensure_ascii=False, indent=1) + "\n")
        print(key, json.dumps(summary), flush=True)


if __name__ == "__main__":
    main()
