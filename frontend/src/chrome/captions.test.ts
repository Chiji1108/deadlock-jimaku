import { describe, expect, it } from "vitest";
import type { Segment } from "../generated/Segment";
import { Captions } from "./captions";
import type { Translator } from "./speech";

const hello: Translator = { translate: (text) => Promise.resolve(`en:${text}`) };
const broken: Translator = { translate: () => Promise.reject(new Error("x")) };

function setup(translator: Translator) {
  const sent: Segment[] = [];
  const captions = new Captions("ja", new Map([["en", translator]]), async (s) => {
    sent.push(s);
  });
  const settled = () => new Promise((resolve) => setTimeout(resolve));
  return { sent, captions, settled };
}

describe("Captions", () => {
  it("途中の文を訳しながら送り、確定したら訳と一緒に閉じる", async () => {
    const { sent, captions, settled } = setup(hello);
    captions.onText("さあ", false);
    await settled();
    captions.onText("さあ始まりました", true);
    await settled();
    expect(sent.map((s) => s.stage)).toEqual([
      "partial",
      "partial",
      "partial",
      "final",
      "enriched",
    ]);
    expect(sent[1]!.draft).toEqual({ en: [{ t: "text", text: "en:さあ" }] });
    const last = sent.at(-1)!;
    expect(new Set(sent.map((s) => s.id)).size).toBe(1);
    expect(last.draft).toBeUndefined();
    expect(last.settledAt).toBeDefined();
    expect(last.enriched!.translations.en).toEqual([{ t: "text", text: "en:さあ始まりました" }]);
  });

  it("確定のたびに次の字幕にする", async () => {
    const { sent, captions, settled } = setup(hello);
    captions.onText("一", true);
    captions.onText("二", true);
    await settled();
    expect(new Set(sent.map((s) => s.id)).size).toBe(2);
  });

  it("訳せなくても話した言葉は残す", async () => {
    const { sent, captions, settled } = setup(broken);
    captions.onText("さあ", true);
    await settled();
    expect(sent.at(-1)!.enriched).toEqual({
      source: [{ t: "text", text: "さあ" }],
      translations: {},
    });
  });

  it("確定した文が空なら、途中の字幕を消す", async () => {
    const { sent, captions, settled } = setup(hello);
    captions.onText("さあ", false);
    captions.onText(" ", true);
    await settled();
    expect(sent.at(-1)!.stage).toBe("discarded");
  });

  it("止めたら確定しなかった字幕を消す", async () => {
    const { sent, captions, settled } = setup(hello);
    captions.onText("さあ", false);
    captions.discard();
    await settled();
    expect(sent.some((s) => s.stage === "discarded")).toBe(true);
  });
});
