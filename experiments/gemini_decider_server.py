"""Google の Gemini Decisions API（gemini-decision-1）を、Jev と同じ形の口で待たせる（DESIGN.md 9.7）。
split_eval の terms-jev に `local-gemini-decision-1` のようなモデル名を渡すと、ここに聞きに来る。
問いの形（名前ごとの `type`・`instructions`・`criteria`）と答えの形（`answers[名前].choice`）は Jev と同じで、
違うのは文の渡し方（`state` ではなく `input` の text）だけ。

usage: python3 gemini_decider_server.py
  GEMINI_API_KEY を使う。終わるときに、使ったトークンと料金（入力 100 万トークン $0.10、出力は無料）を出す
"""

import json
import os
import sys
import urllib.request
from http.server import BaseHTTPRequestHandler, HTTPServer

PORT = 8765
URL = "https://generativelanguage.googleapis.com/v1beta/decisions"
MODEL = "gemini-decision-1"
USD_PER_MILLION = 0.10
tokens = 0


class Handler(BaseHTTPRequestHandler):
    def do_POST(self) -> None:
        global tokens
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        state = body["state"]
        text = state if isinstance(state, str) else json.dumps(state, ensure_ascii=False)
        request = urllib.request.Request(
            URL,
            data=json.dumps({
                "model": MODEL,
                "input": [{"type": "text", "text": text}],
                "questions": body["questions"],
            }).encode(),
            headers={"x-goog-api-key": os.environ["GEMINI_API_KEY"], "Content-Type": "application/json"},
        )
        try:
            with urllib.request.urlopen(request, timeout=30) as res:
                answer = json.loads(res.read())
            used = answer["usage"]["total_input_tokens"]
            tokens += used
            out, status = {"answers": answer["answers"], "usage": {"input_tokens": used, "output_tokens": 0}}, 200
        except urllib.error.HTTPError as e:
            out, status = {"error": e.read().decode()[:500]}, e.code
            print(out, file=sys.stderr, flush=True)
        data = json.dumps(out).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *_: object) -> None:
        pass


if __name__ == "__main__":
    print(f"listening on {PORT}", flush=True)
    try:
        HTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
    except KeyboardInterrupt:
        print(f"{tokens} input tokens, ${tokens * USD_PER_MILLION / 1e6:.4f}", flush=True)
