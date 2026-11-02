<script lang="ts">
  import { onMount } from "svelte";
  import { flip } from "svelte/animate";
  import { SvelteMap } from "svelte/reactivity";
  import { fade, fly } from "svelte/transition";
  import type { Segment } from "../../generated/Segment";
  import Rich from "../../lib/Rich.svelte";
  import { connect } from "../../lib/ws";
  import { params } from "./params";

  const segments = new SvelteMap<string, Segment>();
  let now = $state(Date.now());

  /** 表示時間は settledAt から数える。partial / final の間は消さない */
  const expiresAt = (s: Segment) =>
    s.settledAt === undefined
      ? Infinity
      : s.settledAt + (params.ttl + [...s.text].length * params.perChar) * 1000;

  const isLive = (s: Segment) => s.stage !== "discarded" && expiresAt(s) > now;

  const visible = $derived(
    [...segments.values()]
      // 原文を出さない設定なら、訳文が届くまで何も出さない
      .filter((s) => isLive(s) && (params.source || s.enriched))
      .toSorted((a, b) => a.startedAt - b.startedAt)
      .slice(-params.maxItems),
  );

  onMount(() => {
    const tick = setInterval(() => {
      now = Date.now();
      for (const [id, s] of segments) if (!isLive(s)) segments.delete(id);
    }, 250);
    const disconnect = connect(["subtitle"], (msg) => {
      switch (msg.type) {
        case "snapshot":
          segments.clear();
          for (const s of msg.segments) segments.set(s.id, s);
          break;
        case "segment.upsert":
          segments.set(msg.segment.id, msg.segment);
          break;
      }
    });
    return () => {
      clearInterval(tick);
      disconnect();
    };
  });
</script>

<ol class="captions">
  {#each visible as s (s.id)}
    <li
      class="caption {s.stage}"
      animate:flip={{ duration: 200 }}
      in:fly={{ y: 16, duration: 200 }}
      out:fade={{ duration: 400 }}
    >
      {#if params.source}
        <p class="source" lang={s.lang}>
          {#if s.enriched}<Rich
              tokens={s.enriched.source}
              furigana={params.furigana}
            />{:else}{s.text}{/if}
        </p>
      {/if}
      {#if s.enriched}
        {#each params.langs ?? Object.keys(s.enriched.translations) as lang (lang)}
          {@const tokens = s.enriched.translations[lang]}
          {#if tokens}
            <p class="translation" {lang}><Rich {tokens} /></p>
          {/if}
        {/each}
      {/if}
    </li>
  {/each}
</ol>

<style>
  /* OBS の「カスタム CSS」で上書きできるよう、見た目は変数にまとめる */
  :global(:root) {
    --source-size: 40px;
    --translation-size: 26px;
    --text-color: #fff;
    --pending-color: #c8c8c8;
    --outline-color: rgb(0 0 0 / 0.85);
    --gap: 12px;
  }
  :global(html, body) {
    margin: 0;
    background: transparent;
    overflow: hidden;
  }
  .captions {
    position: fixed;
    inset: auto 0 0 0;
    margin: 0;
    padding: 0 24px 16px;
    list-style: none;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--gap);
    text-align: center;
    color: var(--text-color);
    -webkit-text-stroke: 0.18em var(--outline-color);
    paint-order: stroke fill;
    text-shadow: 0 2px 6px rgb(0 0 0 / 0.6);
  }
  p {
    margin: 0;
    line-height: 1.5;
    font-weight: 700;
  }
  .source {
    font-size: var(--source-size);
  }
  .translation {
    font-size: var(--translation-size);
  }
  /* 確定前（partial / final）は色を落とす */
  .partial .source,
  .final .source {
    color: var(--pending-color);
  }
</style>
