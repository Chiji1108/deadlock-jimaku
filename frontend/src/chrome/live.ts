import type { LiveText } from "../generated/LiveText";
import type { Token } from "../generated/Token";
import type { Translator } from "./speech";

/** ハイブリッドモード。途中の文をその場で訳してアプリへ送る。確定の字幕はアプリが作る */
export class Live {
  private text = "";
  private translations: LiveText["translations"] = {};
  private translating = false;
  /** 区切ったら、訳している途中の文の結果は捨てる */
  private generation = 0;

  constructor(
    private translators: Map<string, Translator>,
    private send: (live: LiveText) => unknown,
  ) {}

  onText(text: string) {
    this.text = text.trim();
    if (!this.text) return;
    this.publish();
    void this.translate();
  }

  /** 発話が閉じた */
  cut() {
    this.generation++;
    this.text = "";
    this.translations = {};
  }

  private publish() {
    this.send({ text: this.text, translations: this.translations });
  }

  private async translate() {
    if (this.translating) return;
    this.translating = true;
    let done = "";
    while (this.text && this.text !== done) {
      const { generation } = this;
      done = this.text;
      const translations = await translateAll(this.translators, done);
      if (generation === this.generation && Object.keys(translations).length > 0) {
        this.translations = translations;
        this.publish();
      }
    }
    this.translating = false;
  }
}

/** 訳せなかった言語は省く */
export async function translateAll(translators: Map<string, Translator>, text: string) {
  const results = await Promise.all(
    [...translators].map(async ([lang, translator]) => {
      const translated = await translator.translate(text).catch(() => "");
      return [lang, tokens(translated)] as const;
    }),
  );
  return Object.fromEntries(results.filter(([, t]) => t.length > 0));
}

export const tokens = (text: string): Token[] => (text ? [{ t: "text", text }] : []);
