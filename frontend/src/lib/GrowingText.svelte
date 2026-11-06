<script lang="ts">
  // 伸びていく字幕。増えた文字だけをその場でフェードさせる（DESIGN.md 11 章）。
  // 1 文字ずつ要素に分けると縁取りが隣の文字に重なり、字間も崩れるので、
  // フェードが終わった文字は 1 つの文字列に戻す
  let { text }: { text: string } = $props();

  const FADE_MS = 160;
  /** 文字列に戻した長さ */
  let settled = $state(0);
  const kept = $derived(Math.min(settled, text.length));

  $effect(() => {
    const { length } = text;
    const timer = setTimeout(() => (settled = length), FADE_MS);
    return () => clearTimeout(timer);
  });
</script>

<span
  >{text.slice(0, kept)}{#if kept < text.length}<span class="fresh">{text.slice(kept)}</span
    >{/if}</span
>

<style>
  .fresh {
    animation: appear 160ms ease-out;
  }
  @keyframes appear {
    from {
      opacity: 0;
    }
  }
</style>
