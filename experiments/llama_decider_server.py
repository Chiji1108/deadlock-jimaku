"""PC の中で動く汎用の小さい LLM（llama.cpp の llama-server）を、判定モデルと同じ形の口で待たせる（DESIGN.md 9.7）。
split_eval の terms-jev に `local-…` のモデル名を渡すと、ここに聞きに来る。ふりがなの読み分け（furigana_eval を
`FURIGANA_EVAL_API=http://127.0.0.1:8765/v1` で動かす）も、同じ口で受ける。

問いごとに、選択肢に A、B、… の記号を振って、記号 1 文字だけを答えさせる（llama.cpp の文法で、ほかの字を書けなくする）。
答えの形は Jev と同じ（`answers[名前].choice` に、選ばれた選択肢の値）。

usage: python3 llama_decider_server.py [LLAMA_URL]
  先に llama-server を立てておく（例: llama-server -m MODEL.gguf --port 8766 -ngl 99 -c 8192 -np 4）
"""

import json
import sys
import time
import urllib.request
from concurrent.futures import ThreadPoolExecutor
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = 8765
LLAMA = sys.argv[1] if len(sys.argv) > 1 else "http://127.0.0.1:8766"
LETTERS = "ABCDEFGHIJKLMNOPQRSTUVWXYZ"
import os

DEBUG = bool(os.environ.get("DECIDER_DEBUG"))
SYSTEM = (
    "You answer one multiple-choice question about a Japanese subtitle from a live stream. "
    "Read the text and the options carefully, then reply with the letter of the single best option and nothing else."
)
tokens = 0
times: list[float] = []


def ask(state: str, question: dict) -> tuple[str | None, int]:
    options = list(question["criteria"].items())[: len(LETTERS)]
    listed = "\n".join(f"{LETTERS[i]}. {description}" for i, (_, description) in enumerate(options))
    grammar = "root ::= " + " | ".join(f'"{LETTERS[i]}"' for i in range(len(options)))
    body = {
        "messages": [
            {"role": "system", "content": SYSTEM},
            {"role": "user", "content": f"{state}\n\nQuestion: {question['instructions']}\n\nOptions:\n{listed}"},
        ],
        "temperature": 0,
        "max_tokens": 2,
        "grammar": grammar,
        "cache_prompt": True,
        # 答える前に考えさせない（考えを本文に書くモデルがある。字幕には遅すぎる）
        "chat_template_kwargs": {"enable_thinking": False},
    }
    request = urllib.request.Request(
        f"{LLAMA}/v1/chat/completions",
        data=json.dumps(body).encode(),
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(request, timeout=60) as res:
        answer = json.loads(res.read())
    message = answer["choices"][0]["message"]
    letter = (message.get("content") or "").strip()[:1]
    if DEBUG:
        print(json.dumps({"q": question["instructions"][:80], "options": [d[:40] for _, d in options], "message": message}, ensure_ascii=False), file=sys.stderr, flush=True)
    used = answer.get("usage", {}).get("prompt_tokens", 0)
    if letter not in LETTERS[: len(options)]:
        return None, used
    return options[LETTERS.index(letter)][0], used


class Handler(BaseHTTPRequestHandler):
    def do_POST(self) -> None:
        global tokens
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        state = body["state"]
        text = state if isinstance(state, str) else json.dumps(state, ensure_ascii=False)
        names = list(body["questions"])
        started = time.monotonic()
        with ThreadPoolExecutor(max_workers=4) as pool:
            results = list(pool.map(lambda name: ask(text, body["questions"][name]), names))
        times.append(time.monotonic() - started)
        used = sum(n for _, n in results)
        tokens += used
        answers = {name: {"type": "choice", "choice": choice} for name, (choice, _) in zip(names, results, strict=True)}
        data = json.dumps({"answers": answers, "usage": {"input_tokens": used, "output_tokens": 0}}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *_: object) -> None:
        pass


if __name__ == "__main__":
    print(f"listening on {PORT}, asking {LLAMA}", flush=True)
    try:
        ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
    except KeyboardInterrupt:
        ordered = sorted(times)
        median = ordered[len(ordered) // 2] if ordered else 0
        print(f"{len(times)} requests, median {median * 1000:.0f} ms, {tokens} input tokens", flush=True)
