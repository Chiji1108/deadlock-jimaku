<script lang="ts">
  import { onMount } from "svelte";
  import { SvelteMap } from "svelte/reactivity";
  import { fade, fly } from "svelte/transition";
  import type { Segment } from "../../generated/Segment";
  import type { SubtitleDisplay } from "../../generated/SubtitleDisplay";
  import Rich from "../../lib/Rich.svelte";
  import { loadGlossary } from "../../lib/glossary.svelte";
  import { slide } from "../../lib/slide";
  import { connect } from "../../lib/ws";

  /** 長い文ほど長く出す（1 文字あたりの秒数） */
  const PER_CHAR_SECONDS = 0.15;

  const segments = new SvelteMap<string, Segment>();
  let now = $state(Date.now());
  /** 見せ方は操作パネルで変え、サーバーから届く（接続直後にも届く） */
  let display = $state<SubtitleDisplay | undefined>();

  /** 表示時間は settledAt から数える。partial / final の間は消さない */
  const expiresAt = (s: Segment) =>
    s.settledAt === undefined || !display
      ? Infinity
      : s.settledAt + (display.holdSeconds + [...s.text].length * PER_CHAR_SECONDS) * 1000;

  const isLive = (s: Segment) => s.stage !== "discarded" && expiresAt(s) > now;

  const visible = $derived(
    display
      ? [...segments.values()]
          // 原文を出さない設定なら、訳文が届くまで何も出さない
          .filter((s) => isLive(s) && (display!.showSource || s.enriched))
          .toSorted((a, b) => a.startedAt - b.startedAt)
          .slice(-display.maxLines)
      : [],
  );

  onMount(() => {
    const tick = setInterval(() => {
      now = Date.now();
      for (const [id, s] of segments) if (!isLive(s)) segments.delete(id);
    }, 250);
    const disconnect = connect(["subtitle", "glossary", "display"], (msg) => {
      switch (msg.type) {
        case "snapshot":
          segments.clear();
          for (const s of msg.segments) segments.set(s.id, s);
          break;
        case "segment.upsert":
          segments.set(msg.segment.id, msg.segment);
          break;
        // 辞書ができたとき（と接続直後）に届く
        case "glossary.updated":
          void loadGlossary();
          break;
        case "display":
          display = msg.subtitles;
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
      animate:slide={{ duration: 200 }}
      in:fly={{ y: 16, duration: 200 }}
      out:fade={{ duration: 400 }}
    >
      {#if display?.showSource}
        <p class="source" lang={s.lang}>
          {#if s.enriched}<Rich
              tokens={s.enriched.source}
              lang={s.lang}
              furigana={display.furigana}
            />{:else}{s.text}{/if}
        </p>
      {/if}
      {#if s.enriched}
        {#each Object.entries(s.enriched.translations) as [lang, tokens] (lang)}
          <p class="translation" {lang}><Rich {tokens} {lang} furigana={display?.furigana} /></p>
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
