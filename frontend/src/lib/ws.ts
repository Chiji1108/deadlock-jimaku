import type { ClientMsg } from "../generated/ClientMsg";
import type { ServerMsg } from "../generated/ServerMsg";
import type { Topic } from "../generated/Topic";

export type Status = "connecting" | "open" | "closed";

/**
 * /ws に接続してトピックを購読する。切れたら指数バックオフで再接続し、
 * 再接続後はサーバーが送る snapshot で状態を置き換える（DESIGN.md 10 章）。
 * 戻り値は切断する関数。
 */
export function connect(
  topics: Topic[],
  onMessage: (msg: ServerMsg) => void,
  onStatus: (status: Status) => void = () => {},
): () => void {
  let ws: WebSocket | undefined;
  let retry = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let stopped = false;

  const open = () => {
    onStatus("connecting");
    const socket = new WebSocket(
      `${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/ws`,
    );
    ws = socket;
    socket.addEventListener("open", () => {
      retry = 0;
      onStatus("open");
      socket.send(JSON.stringify({ type: "subscribe", topics } satisfies ClientMsg));
    });
    socket.addEventListener("message", (e) => onMessage(JSON.parse(e.data) as ServerMsg));
    socket.addEventListener("close", () => {
      onStatus("closed");
      if (!stopped) timer = setTimeout(open, Math.min(500 * 2 ** retry++, 10_000));
    });
  };

  open();
  return () => {
    stopped = true;
    clearTimeout(timer);
    ws?.close();
  };
}
