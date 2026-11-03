# /// script
# requires-python = ">=3.12"
# dependencies = ["anthropic"]
# ///
"""日本のプレイヤーが Deadlock の用語を口でどう呼ぶか（通称・略称・昔の名前・英語名のカタカナ読み）を、
Claude と Web 検索で調べて候補を出典付きで書き出す（DESIGN.md 8.4）。

候補は人が出典を見て確かめてから data/aliases.ja.json に入れる。書き出しは追記なので、
止めても続きから調べられる（調べ終えた参照は飛ばす）。

usage: uv run aliases_research.py GLOSSARY_JSON [--only REF,REF] [--limit N]
  GLOSSARY_JSON  アプリのデータディレクトリの assets/v6/{clientVersion}/glossary.json
  出力           results/aliases-candidates.ja.json
"""

import argparse
import json
from pathlib import Path

import anthropic

MODEL = "claude-opus-5-5"
# 1 回の問い合わせで調べる用語の数（検索が多くなりすぎない大きさ）
PER_REQUEST = 8
OUT = Path(__file__).parent / "results" / "aliases-candidates.ja.json"

SYSTEM = """You are researching how Japanese players of Valve's game Deadlock actually say game terms out loud, \
for a live-caption tool that turns spoken nicknames into the official term.

For each term you get the official English name and the official Japanese localization. Find, with web search:
- community abbreviations and nicknames used by Japanese players (e.g. on wikiwiki.jp/deadlockjp, gamerch, \
YouTube video titles, X posts, Japanese streams)
- former official Japanese or English names from earlier patches that players still use
- the English name written in katakana, when it differs from the official Japanese name
- general FPS / gaming slang that Japanese players use for the same thing in other games (e.g. 拡マガ for an \
extended magazine, as in Apex Legends). A page showing that usage in another game is a valid source.

Rules:
- Only include forms you actually found in use, with the URL of a page that shows it. Do not guess.
- Write each alias as speech recognition would write it. When an alias mixes kanji and kana, also give the \
katakana-only form as a separate alias (e.g. 拡マガ and カクマガ).
- Never include the official Japanese name itself, ordinary words that would match unrelated speech, or forms \
shorter than 2 characters.

When you have researched every term, call record_aliases once with all of them (an empty list for terms \
with nothing found)."""

RECORD = {
    "name": "record_aliases",
    "description": "Record the aliases found for every term in this request, each with its source URL.",
    "strict": True,
    "input_schema": {
        "type": "object",
        "additionalProperties": False,
        "required": ["terms"],
        "properties": {
            "terms": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": False,
                    "required": ["ref", "aliases"],
                    "properties": {
                        "ref": {"type": "string"},
                        "aliases": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "additionalProperties": False,
                                "required": ["alias", "source"],
                                "properties": {
                                    "alias": {"type": "string"},
                                    "source": {"type": "string"},
                                },
                            },
                        },
                    },
                },
            }
        },
    },
}
TOOLS = [
    {"type": "web_search_20260209", "name": "web_search"},
    {"type": "web_fetch_20260209", "name": "web_fetch"},
    RECORD,
]


def research(client: anthropic.Anthropic, rows: list[dict]) -> tuple[list[dict], dict]:
    """rows を調べて record_aliases の中身を返す。検索の途中で止まったら（pause_turn）続けさせる"""
    messages = [{"role": "user", "content": json.dumps(rows, ensure_ascii=False)}]
    usage = {"input": 0, "output": 0, "searches": 0}
    for _ in range(6):
        with client.messages.stream(
            model=MODEL,
            max_tokens=32000,
            system=SYSTEM,
            tools=TOOLS,
            output_config={"effort": "medium"},
            messages=messages,
        ) as stream:
            message = stream.get_final_message()
        usage["input"] += message.usage.input_tokens
        usage["output"] += message.usage.output_tokens
        server = getattr(message.usage, "server_tool_use", None)
        usage["searches"] += getattr(server, "web_search_requests", 0) or 0
        for block in message.content:
            if block.type == "tool_use" and block.name == "record_aliases":
                return block.input["terms"], usage
        if message.stop_reason != "pause_turn":
            raise RuntimeError(f"stopped without record_aliases ({message.stop_reason})")
        messages.append({"role": "assistant", "content": message.content})
    raise RuntimeError("too many pause_turn")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("glossary", type=Path)
    parser.add_argument("--only", help="調べる参照（カンマ区切り）")
    parser.add_argument("--limit", type=int, help="調べる用語の数の上限（試すとき）")
    args = parser.parse_args()

    entities = json.loads(args.glossary.read_text())["entities"]
    done = json.loads(OUT.read_text()) if OUT.exists() else {}
    refs = args.only.split(",") if args.only else list(entities)
    rows = [
        {"ref": ref, "kind": e["kind"], "en": e["names"].get("en"), "ja": e["names"].get("ja")}
        for ref in refs
        if ref not in done and (e := entities.get(ref)) and e["names"].get("ja")
    ][: args.limit]
    print(f"{len(rows)} terms to research ({len(done)} already done)")

    client = anthropic.Anthropic()
    total = {"input": 0, "output": 0, "searches": 0}
    for i in range(0, len(rows), PER_REQUEST):
        chunk = rows[i : i + PER_REQUEST]
        try:
            terms, usage = research(client, chunk)
        except Exception as e:  # 次の回で調べ直す
            print(f"  failed {[r['ref'] for r in chunk]}: {e}")
            continue
        found = {t["ref"]: t["aliases"] for t in terms}
        for row in chunk:
            done[row["ref"]] = {"en": row["en"], "ja": row["ja"], "aliases": found.get(row["ref"], [])}
            if found.get(row["ref"]):
                print(f"  {row['ja']}: " + ", ".join(a["alias"] for a in found[row["ref"]]))
        OUT.parent.mkdir(exist_ok=True)
        OUT.write_text(json.dumps(done, ensure_ascii=False, indent=2))
        for k in total:
            total[k] += usage[k]
        cost = total["input"] * 4e-6 + total["output"] * 20e-6 + total["searches"] * 0.01
        print(f"  [{min(i + PER_REQUEST, len(rows))}/{len(rows)}] {total} ≈ ${cost:.2f}")


main()
