import { describe, expect, it } from "vitest";
import type { LiveText } from "../generated/LiveText";
import { Live } from "./live";
import type { Translator } from "./speech";

const hello: Translator = { translate: (text) => Promise.resolve(`en:${text}`) };
const settled = () => new Promise((resolve) => setTimeout(resolve));

describe("Live", () => {
  it("文をすぐ送り、訳せたらもう一度送る", async () => {
    const sent: LiveText[] = [];
    const live = new Live(new Map([["en", hello]]), (l) => sent.push(l));
    live.onText("さあ", false);
    await settled();
    expect(sent).toEqual([
      { text: "さあ", translations: {} },
      { text: "さあ", translations: { en: [{ t: "text", text: "en:さあ" }] } },
    ]);
  });

  it("発話の途中で Chrome が確定しても、前の文を消さない", () => {
    const sent: LiveText[] = [];
    const live = new Live(new Map(), (l) => sent.push(l));
    live.onText("さあ始まりました", true);
    live.onText("今日は", false);
    expect(sent.at(-1)!.text).toBe("さあ始まりました今日は");
  });

  it("区切ったら、前の文の訳を次の文に使わない", async () => {
    const sent: LiveText[] = [];
    const live = new Live(new Map([["en", hello]]), (l) => sent.push(l));
    live.onText("前の文", false);
    live.cut();
    await settled();
    live.onText("次", false);
    expect(sent.at(-1)).toEqual({ text: "次", translations: {} });
  });
});
