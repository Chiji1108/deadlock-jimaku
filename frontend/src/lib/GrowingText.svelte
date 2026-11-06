<script lang="ts">
  // 伸びていく字幕。認識の結果はまとめて届くので、数文字ずつ順に出して滑らかに見せる（DESIGN.md 11 章）
  let { text }: { text: string } = $props();

  const STEP_MS = 30;
  /** 届いた分を約 0.2 秒で出し切る */
  const STEPS = 6;
  let shown = $state(0);
  const visible = $derived(Math.min(shown, text.length));

  $effect(() => {
    const rest = text.length - visible;
    if (rest <= 0) return;
    const timer = setTimeout(() => (shown = visible + Math.ceil(rest / STEPS)), STEP_MS);
    return () => clearTimeout(timer);
  });
</script>

<span>{text.slice(0, visible)}</span>
