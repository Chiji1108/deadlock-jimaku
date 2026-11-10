"""用語の判定の文と正解（data/terms_cases.json。人が書く）を、辞書の参照に直して results/terms_cases.json に書く。

正解は [話した言葉, "種類:英語名"]。スキルは "ability:ヒーローの英語名#番号" とも書ける。
出力は entity_llm.json と同じ形（`llm[0].answer.segments`）で、terms_eval と split_eval の `--cases` に渡す。

usage: python3 experiments/terms_cases.py GLOSSARY.json
  （アプリの保存した辞書。`{データディレクトリ}/assets/v9/{版}/glossary.json`）
"""

import json
import sys
from pathlib import Path

HERE = Path(__file__).parent
entities = json.loads(Path(sys.argv[1]).read_text())["entities"]


def find(spec: str) -> str:
    kind, name = spec.split(":", 1)
    if "#" in name:
        hero, slot = name.split("#")
        owner = find(f"hero:{hero}")
        found = [r for r, e in entities.items() if e.get("hero") == owner and e.get("abilitySlot") == int(slot)]
    else:
        found = [r for r, e in entities.items() if e["kind"] == kind and e["names"].get("en") == name]
    if len(found) != 1:
        raise SystemExit(f"{spec}: {len(found)} 件")
    return found[0]


cases = []
for case in json.loads((HERE / "data/terms_cases.json").read_text()):
    text, at, segments = case["text"], 0, []
    for spoken, spec in case["terms"]:
        start = text.find(spoken, at)
        if start < 0:
            raise SystemExit(f"{text}: 「{spoken}」がない")
        if start > at:
            segments.append({"text": text[at:start], "ref": None})
        segments.append({"text": spoken, "ref": find(spec)})
        at = start + len(spoken)
    if at < len(text):
        segments.append({"text": text[at:], "ref": None})
    cases.append({
        "text": text,
        "context": case.get("context", []),
        "group": case["group"],
        "llm": [{"answer": {"segments": segments}}],
    })

out = HERE / "results/terms_cases.json"
out.write_text(json.dumps(cases, ensure_ascii=False, indent=1) + "\n")
print(f"{len(cases)} 文を {out} に書いた")
