<script lang="ts">
  // アイテム説明のオーバーレイ（DESIGN.md 12.5）。字幕でアイテム名が出たら、その説明のカードを
  // 1 枚ずつ出す。カードは deadlock-ui（Web Components）の dl-item-tooltip で、データは
  // 部品が deadlock-api から取る
  import { defineCustomElement as defineProvider } from "@deadlock-api/ui-core/dist/components/dl-provider.js";
  import { defineCustomElement as defineTooltip } from "@deadlock-api/ui-core/dist/components/dl-item-tooltip.js";
  import { SUPPORTED_LANGUAGES } from "@deadlock-api/ui-core/dist/components/index.js";
  import { onMount } from "svelte";
  import { fade, fly } from "svelte/transition";
  import type { ItemCardsView } from "../../generated/ItemCardsView";
  import type { Segment } from "../../generated/Segment";
  import { glossary, loadGlossary } from "../../lib/glossary.svelte";
  import { connect } from "../../lib/ws";

  defineProvider();
  defineTooltip();

  /** 同じアイテムを続けて話しても、この間は出し直さない */
  const COOLDOWN_MS = 60_000;

  let view = $state<ItemCardsView | undefined>();
  /** 今出しているカード（アイテムの class_name） */
  let current = $state<string | undefined>();
  const queue: string[] = [];
  const shownAt = new Map<string, number>();
  /** 説明を出し終えた字幕（同じ字幕の更新で出し直さない） */
  const handled = new Set<string>();
  let timer: ReturnType<typeof setTimeout> | undefined;

  /** deadlock-ui が対応していない言語は英語で出す */
  const language = $derived(
    view && (SUPPORTED_LANGUAGES as string[]).includes(view.language) ? view.language : "english",
  );

  /** 翻訳まで済んだ字幕から、話した順にアイテムを拾う */
  function onSegment(segment: Segment) {
    if (!segment.enriched || handled.has(segment.id)) return;
    handled.add(segment.id);
    const now = Date.now();
    for (const token of segment.enriched.source) {
      if (token.t !== "entity") continue;
      const entity = glossary.entity(token.ref);
      if (entity?.kind !== "item") continue;
      const item = entity.className;
      if (now - (shownAt.get(item) ?? -Infinity) < COOLDOWN_MS || queue.includes(item)) continue;
      shownAt.set(item, now);
      queue.push(item);
    }
    if (!current) next();
  }

  function next() {
    current = queue.shift();
    if (current) timer = setTimeout(next, (view?.holdSeconds ?? 8) * 1000);
  }

  onMount(() => {
    const disconnect = connect(["subtitle", "glossary", "display"], (msg) => {
      switch (msg.type) {
        // 接続した時点で出ていた字幕は、説明済みとして扱う（つなぎ直すたびに出し直さない）
        case "snapshot":
          for (const s of msg.segments) handled.add(s.id);
          break;
        case "segment.upsert":
          onSegment(msg.segment);
          break;
        case "glossary.updated":
          void loadGlossary();
          break;
        case "display":
          view = msg.itemCards;
          break;
      }
    });
    return () => {
      clearTimeout(timer);
      disconnect();
    };
  });
</script>

{#if view}
  <dl-provider {language}>
    {#key current}
      {#if current}
        <div class="card" in:fly={{ x: -24, duration: 250 }} out:fade={{ duration: 300 }}>
          <dl-item-tooltip class-name={current}></dl-item-tooltip>
        </div>
      {/if}
    {/key}
  </dl-provider>
{/if}

<style>
  :global(html, body) {
    margin: 0;
    background: transparent;
    overflow: hidden;
  }
  /* 入れ替わるときに前のカードと重ねる */
  .card {
    position: absolute;
    top: 0;
    left: 0;
  }
</style>
