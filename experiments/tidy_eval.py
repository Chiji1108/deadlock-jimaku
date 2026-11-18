"""確定の文の数字と句読点を整える工程（`crates/core/src/tidy.rs`。DESIGN.md 9.15）を測る。
アプリと同じ処理（`cargo run --release -p overlays-core --example tidy`）に文を通して、正解の文と比べる。

句読点: 句読点と空白を除いた文どうしを文字で突き合わせ、文の中の区切り（「、」か「。」）の位置が合うか、
合った位置で「、」と「。」の種類が合うか、終わりに「。」か「？」があるかを数える。
- chrome: 本物の声（data/sample01、02）を Chrome が書いた確定の文（results/sample0N.chrome.json。話の間に空白が入る）
- chrome-tts: ElevenLabs の 126 発話を、声ごとに 1 本につないで Chrome に聞かせた文（results/stt_terms.json の chrome）
- k2: ElevenLabs の 126 発話（data/tts）を ReazonSpeech k2 が書いた文（results/stt_terms.json。句読点も空白もない）
- oracle: 台本の文の句読点を空白に置き換えたもの（話の間が完全にわかるとき、「、」と「。」を選べるか）
数字: data/tidy_script.json の 36 文で、台本の算用数字を漢数字に書き換えた文（oracle。決まりそのものを確かめる）と、録音（data/tts_tidy。`tts_generate.py --script data/tidy_script.json --out data/tts_tidy`）を
k2 が書いた文で、正解の算用数字が書けたか、変えてはいけない言い回し（一気、一緒、一瞬、一番）が残ったかを数える。

usage: python3 tidy_eval.py [--k2-tidy results/tidy_k2.json]
  `--k2-tidy` は、tts_tidy の録音を k2 で書いた文（stt_eval_k2_tidy の出力）。なければ数字は測らない
"""

import difflib
import json
import re
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).parent
MARKS = "、。！？!?"


def tidy(texts: list[str]) -> list[str]:
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False, encoding="utf-8") as f:
        json.dump(texts, f, ensure_ascii=False)
    out = subprocess.run(
        ["cargo", "run", "-q", "--release", "-p", "overlays-core", "--example", "tidy", "--", f.name],
        check=True, capture_output=True, text=True, cwd=HERE.parent,
    ).stdout
    return json.loads(out)


def split(text: str) -> tuple[str, dict[int, str]]:
    """句読点と空白を除いた文と、何文字目の後ろにどの区切りがあるか（「、」か「。」。！と？は「。」）"""
    chars, marks = [], {}
    for c in text:
        if c in MARKS:
            if chars:
                marks[len(chars)] = "、" if c == "、" else "。"
        elif not c.isspace():
            chars.append(c)
    return "".join(chars), marks


def kanji(n: int) -> str:
    """算用数字を漢数字に（35 → 三十五、15000 → 一万五千）"""
    digits = "〇一二三四五六七八九"

    def small(n: int) -> str:
        out = ""
        for unit, name in ((1000, "千"), (100, "百"), (10, "十")):
            q, n = divmod(n, unit)
            if q:
                out += ("" if q == 1 else digits[q]) + name
        return out + (digits[n] if n else "")

    man, rest = divmod(n, 10000)
    return ((small(man) + "万") if man else "") + small(rest) or "〇"


def to_kanji(text: str) -> str:
    def one(m):
        a, _, b = m.group(0).partition("万")
        return kanji(int(a) * 10000 + int(b or 0)) if "万" in m.group(0) else kanji(int(a))

    return re.sub(r"\d+(?:万\d*)?", one, text)


def count_numbers(rows: list[tuple[str, str]]) -> dict:
    """(正解, 整えた文) の組で、正解の算用数字が書けたか、正解にない数字が出たか、言い回しが残ったか"""
    want = written = extra = idioms = kept = 0
    for ref, hyp in rows:
        nums = re.findall(r"\d+(?:万\d*)?", ref)
        want += len(nums)
        written += sum(n in hyp for n in nums)
        extra += sum(n not in nums for n in re.findall(r"\d+(?:万\d*)?", hyp))
        for idiom in ["一気", "一緒", "一瞬", "一番"]:
            if idiom in ref:
                idioms += 1
                kept += idiom in hyp
    return {"numbers": want, "written": written, "extra": extra, "idioms": idioms, "idiomsKept": kept}


def score(pairs: list[tuple[str, str]]) -> dict:
    """(正解, 整えた文) の組で数える"""
    want = got = hit = same = ends = 0
    for ref, hyp in pairs:
        r, rm = split(ref)
        h, hm = split(hyp)
        # 文字起こしの誤りがあっても、合っている所どうしで位置を対応させる
        to_ref = {}
        for a, b, n in difflib.SequenceMatcher(None, h, r, autojunk=False).get_matching_blocks():
            for k in range(n + 1):
                to_ref[a + k] = b + k
        rm_mid = {k: v for k, v in rm.items() if k < len(r)}
        hm_mid = {to_ref[k]: v for k, v in hm.items() if k < len(h) and k in to_ref}
        want += len(rm_mid)
        got += len(hm_mid)
        hit += len(rm_mid.keys() & hm_mid.keys())
        same += sum(rm_mid[k] == hm_mid[k] for k in rm_mid.keys() & hm_mid.keys())
        ends += len(h) in hm
    n = len(pairs)
    return {
        "sentences": n,
        "marks": want,
        "placed": got,
        "rightPlace": hit,
        "rightKind": same,
        "ended": ends,
        "precision": round(hit / got, 3) if got else None,
        "recall": round(hit / want, 3) if want else None,
    }


def main() -> None:
    report = {}
    script = [l["text"] for l in json.loads((HERE / "data/stt_script.json").read_text())]

    # chrome: 本物の声。1 回目の書き起こしの、確定した文ごとに整えてからつなげる
    chrome_pairs = []
    for name in ["sample01", "sample02"]:
        finals = [f for f in json.loads((HERE / f"results/{name}.chrome.json").read_text())["chrome-web-speech"][0]["finals"] if f.strip()]
        ref = (HERE / f"data/{name}.txt").read_text().strip()
        for label, texts in {"none": finals, "tidy": tidy(finals)}.items():
            report.setdefault(f"chrome/{label}", []).append((ref, "".join(texts)))
        chrome_pairs.append((ref, "".join(tidy(finals))))

    # chrome-tts: ElevenLabs の 126 発話を、声ごとに 1 本につないで Chrome に聞かせた文（3 本）。つないだ台本と比べる
    voices = json.loads((HERE / "results/stt_terms.json").read_text())["chrome"]["runs"]
    joined = "".join(script)
    texts = [v["heard"] for v in voices]
    report["chrome-tts/none"] = [(joined, h) for h in texts]
    report["chrome-tts/tidy"] = [(joined, h) for h in tidy(texts)]

    # k2: ElevenLabs の 126 発話
    runs = json.loads((HERE / "results/stt_terms.json").read_text())["local/reazonspeech-k2-v2@1t+all"]["runs"]
    heard = [r["heard"] for r in runs]
    refs = [script[r["line"]] for r in runs]
    report["k2/none"] = list(zip(refs, heard))
    report["k2/tidy"] = list(zip(refs, tidy(heard)))

    # oracle: 台本の句読点を空白にした文（終わりの句読点は消す）
    spaced = [re.sub(f"[{MARKS}]", " ", t).strip() for t in script]
    report["oracle/tidy"] = list(zip(script, tidy(spaced)))

    summary = {name: score(pairs) for name, pairs in report.items()}
    for name, s in summary.items():
        print(f"{name:14} {json.dumps(s, ensure_ascii=False)}")
    examples = {name: [{"ref": r, "tidy": h} for r, h in pairs[:12]] for name, pairs in report.items() if name.endswith("tidy")}

    # 数字
    tidy_script = [l["text"] for l in json.loads((HERE / "data/tidy_script.json").read_text())]
    spoken = [re.sub(f"[{MARKS}]", "", to_kanji(t)) for t in tidy_script]
    summary["numbers/oracle"] = count_numbers(list(zip(tidy_script, tidy(spoken))))
    print(f"numbers/oracle {json.dumps(summary['numbers/oracle'])}")
    if "--k2-tidy" in sys.argv:
        written = json.loads(Path(sys.argv[sys.argv.index("--k2-tidy") + 1]).read_text())
        rows = [(tidy_script[w["line"]], w["heard"]) for w in written]
        for label, texts in {"none": [h for _, h in rows], "tidy": tidy([h for _, h in rows])}.items():
            summary[f"numbers/{label}"] = count_numbers([(ref, hyp) for (ref, _), hyp in zip(rows, texts)])
            print(f"numbers/{label:6} {json.dumps(summary[f'numbers/{label}'])}")
        examples["numbers/tidy"] = [{"ref": r, "heard": h, "tidy": t} for (r, h), t in zip(rows, tidy([h for _, h in rows]))]
    (HERE / "results/tidy_eval.json").write_text(json.dumps({"summary": summary, "examples": examples}, ensure_ascii=False, indent=1) + "\n")


if __name__ == "__main__":
    main()
