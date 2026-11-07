<script lang="ts">
  import { onMount } from "svelte";
  import { SvelteMap } from "svelte/reactivity";
  import type { Segment } from "../../generated/Segment";
  import type { SubtitleDisplay } from "../../generated/SubtitleDisplay";
  import type { SubtitleStyle } from "../../generated/SubtitleStyle";
  import type { Token } from "../../generated/Token";
  import Captions from "../../lib/Captions.svelte";
  import { api } from "../../lib/api";
  import { loadGlossary } from "../../lib/glossary.svelte";
  import { applyFontRules, fontSample, subtitleFontRules } from "../../lib/overlay-fonts";
  import { isReady, visibleLines } from "../../lib/subtitle-lines";
  import { connect } from "../../lib/ws";

  const segments = new SvelteMap<string, Segment>();
  let now = $state(Date.now());
  /** 見せ方は操作パネルで変え、サーバーから届く（接続直後にも届く） */
  let display = $state<SubtitleDisplay | undefined>();
  let look = $state<SubtitleStyle | undefined>();

  /** 操作パネルのプレビュー（`?preview=ja,en`）では、字幕がなくても言語ごとの見本を出しておく。OBS には出さない */
  const [sampleLang, ...sampleTargets] =
    new URLSearchParams(location.search).get("preview")?.split(",") ?? [];
  const sampleLine = (lang: string): Token[] => [{ t: "text", text: fontSample(lang) }];
  const SAMPLE: Segment[] = sampleLang
    ? [
        {
          id: "sample",
          startedAt: 0,
          stage: "enriched",
          lang: sampleLang,
          text: fontSample(sampleLang),
          enriched: {
            source: sampleLine(sampleLang),
            translations: Object.fromEntries(sampleTargets.map((t) => [t, sampleLine(t)])),
          },
        },
      ]
    : [];

  /** 表示時間は settledAt から数える。partial / final の間は消さない */
  const expiresAt = (s: Segment) =>
    s.settledAt === undefined || !display
      ? Infinity
      : s.settledAt + (display.holdSeconds + [...s.text].length * display.perCharSeconds) * 1000;

  const isLive = (s: Segment) => s.stage !== "discarded" && expiresAt(s) > now;

  const visible = $derived(
    display
      ? visibleLines(
          [...segments.values()]
            .filter((s) => isLive(s) && isReady(s, display!))
            .toSorted((a, b) => a.startedAt - b.startedAt),
          now,
          display,
        )
      : [],
  );

  onMount(() => {
    // OBS の中で開かれたか（はじめかたのチェックに使う）。OBS のドックの操作パネルのプレビューも
    // obsstudio を持つかもしれないので、iframe の中は除く
    if ("obsstudio" in window && window.parent === window)
      void api.updateSettings({ obsSeen: true }).catch(() => {});
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
          applyFontRules(subtitleFontRules(msg.subtitleStyle.languages));
          display = msg.subtitles;
          look = msg.subtitleStyle;
          break;
      }
    });
    return () => {
      clearInterval(tick);
      disconnect();
    };
  });
</script>

<div class="box">
  <Captions segments={visible.length > 0 || !display ? visible : SAMPLE} {display} {look} />
</div>

<style>
  :global(html, body) {
    margin: 0;
    background: transparent;
    overflow: hidden;
  }
  .box {
    position: fixed;
    inset: 0;
  }
</style>
