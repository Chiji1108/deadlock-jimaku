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
  import { placeItems } from "../../lib/anchor";
  import { glossary, loadGlossary } from "../../lib/glossary.svelte";
  import { connect } from "../../lib/ws";

  defineProvider();
  defineTooltip();

  /** 待っているカードの上限。話から大きく遅れないよう、古いものは捨てる */
  const MAX_QUEUE = 3;
  /** 説明を出し終えた字幕として覚えておく数（サーバーが持つ直近の字幕より多ければよい） */
  const MAX_HANDLED = 200;

  let view = $state<ItemCardsView | undefined>();
  /** 今出しているカード（アイテムの class_name） */
  let current = $state<string | undefined>();
  const queue: string[] = [];
  let timer: ReturnType<typeof setTimeout> | undefined;
  /** 説明を出し終えた字幕（同じ字幕の更新で出し直さない） */
  const handled = new Set<string>();
  /** 辞書を読む前に届いた字幕。読めたら拾う */
  const pending = new Map<string, Segment>();

  /** deadlock-ui が対応していない言語は英語で出す */
  const language = $derived(
    view && (SUPPORTED_LANGUAGES as string[]).includes(view.language) ? view.language : "english",
  );

  /** 翻訳まで済んだ字幕から、話した順にアイテムを拾う */
  function onSegment(segment: Segment) {
    if (!segment.enriched || handled.has(segment.id)) return;
    if (!glossary.ready) {
      pending.set(segment.id, segment);
      return;
    }
    handled.add(segment.id);
    if (handled.size > MAX_HANDLED) handled.delete(handled.values().next().value!);
    for (const token of segment.enriched.source) {
      if (token.t !== "entity") continue;
      const entity = glossary.entity(token.ref);
      if (entity?.kind === "item") show(entity.className);
    }
  }

  /** 出しているカードをもう一度話したら、残りの時間を最初に戻す。待っているものは待たせる */
  function show(item: string) {
    if (item === current) return hold();
    if (queue.includes(item)) return;
    queue.push(item);
    if (queue.length > MAX_QUEUE) queue.shift();
    if (!current) next();
  }

  function next() {
    current = queue.shift();
    if (current) hold();
  }

  function hold() {
    clearTimeout(timer);
    timer = setTimeout(next, (view?.holdSeconds ?? 8) * 1000);
  }

  onMount(() => {
    const disconnect = connect(["subtitle", "glossary", "display"], (msg) => {
      switch (msg.type) {
        // 接続した時点で説明まで済んでいた字幕は、説明済みとして扱う（つなぎ直すたびに出し直さない）。
        // 途中の字幕は、説明が付いたときに拾う
        case "snapshot":
          for (const s of msg.segments) if (s.enriched) handled.add(s.id);
          break;
        case "segment.upsert":
          onSegment(msg.segment);
          break;
        case "glossary.updated":
          void loadGlossary().then(() => {
            const waiting = [...pending.values()];
            pending.clear();
            waiting.forEach(onSegment);
          });
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
  <!-- ソースの箱の中で、操作パネルで選んだ位置に寄せる。右に寄せたら右から出す -->
  <dl-provider {language} class="stage" style:place-items={placeItems(view.anchor)}>
    <!-- 1 枚ずつ。入れ替わるときは前のカードと重ねて入れ替える -->
    {#each current ? [current] : [] as item (item)}
      <div
        class="card"
        in:fly={{ x: view.anchor.endsWith("right") ? 24 : -24, duration: 250 }}
        out:fade={{ duration: 300 }}
      >
        <dl-item-tooltip class-name={item}></dl-item-tooltip>
      </div>
    {/each}
  </dl-provider>
{/if}

<style>
  :global(html, body) {
    margin: 0;
    background: transparent;
    overflow: hidden;
  }
  :global(:root) {
    --scale: 1;
  }
  /* カードが箱より大きくても、選んだ位置に寄せる（はみ出した分が切れる） */
  .stage {
    position: fixed;
    inset: 0;
    display: grid;
    grid-template: minmax(0, 1fr) / minmax(0, 1fr);
  }
  /* 入れ替わるときに前のカードと重ねる */
  .card {
    grid-area: 1 / 1;
    zoom: var(--scale);
  }
</style>
