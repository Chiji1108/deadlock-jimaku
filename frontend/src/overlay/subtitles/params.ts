// URL パラメータ（DESIGN.md 11 章）。OBS のブラウザソースごとに変えられる
const q = new URLSearchParams(location.search);

const num = (key: string, fallback: number) => {
  const value = Number(q.get(key));
  return q.has(key) && Number.isFinite(value) ? value : fallback;
};

export const params = {
  /** 表示する訳文の言語。未指定なら届いた言語をすべて表示する */
  langs: q.get("langs")?.split(",").filter(Boolean),
  source: q.get("source") !== "0",
  furigana: q.get("furigana") !== "0",
  maxItems: num("maxItems", 3),
  ttl: num("ttl", 6),
  perChar: num("perChar", 0.15),
};
