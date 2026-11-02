import type { Entity } from "../generated/Entity";
import type { Glossary } from "../generated/Glossary";

let current = $state<Glossary | undefined>();

export const glossary = {
  entity(ref: string): Entity | undefined {
    return current?.entities[ref];
  },
};

/** 起動直後はサーバーが辞書を作っている途中（503）。その場合は glossary.updated を待つ */
export async function loadGlossary() {
  const res = await fetch("/api/glossary");
  if (res.ok) current = (await res.json()) as Glossary;
}

/** その言語の公式名。辞書にない言語なら英語名にする */
export function entityName(entity: Entity, lang: string): string {
  return entity.names[lang] ?? entity.names.en ?? entity.className;
}
