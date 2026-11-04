import type { Entity } from "../generated/Entity";
import type { Glossary } from "../generated/Glossary";

let current = $state<Glossary | undefined>();
let latestRequest = 0;

export const glossary = {
  /** 一度でも読めたか */
  get ready() {
    return current !== undefined;
  },
  entity(ref: string): Entity | undefined {
    return current?.entities[ref];
  },
};

/** サーバーのない LP の見本は、用意した辞書を渡す */
export function setGlossary(next: Glossary) {
  current = next;
}

/**
 * サーバーが glossary.updated を送ってきたら呼ぶ（接続直後にも届く）。
 * 呼び出しが重なったら最後のものだけを反映し、失敗したら今の辞書のまま次の通知を待つ。
 */
export async function loadGlossary() {
  const request = ++latestRequest;
  try {
    const res = await fetch("/api/glossary");
    if (!res.ok) return;
    const next = (await res.json()) as Glossary;
    if (request === latestRequest) current = next;
  } catch (e) {
    console.warn("failed to load glossary", e);
  }
}

/** その言語の公式名。辞書にない言語なら英語名にする */
export function entityName(entity: Entity, lang: string): string {
  return entity.names[lang] ?? entity.names.en ?? entity.className;
}
