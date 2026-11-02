<script lang="ts">
  import { panel } from "../panel.svelte";

  const FLOOR = -60;
  const ratio = $derived(
    panel.level === undefined ? 0 : Math.min(1, Math.max(0, (panel.level - FLOOR) / -FLOOR)),
  );
  const ticks = [-60, -40, -20, -10, 0];
</script>

<div class="flex flex-col gap-1.5">
  <div class="flex items-center gap-3">
    <div
      class="relative h-3 flex-1 overflow-hidden rounded-full bg-muted"
      role="meter"
      aria-label="入力レベル"
      aria-valuemin={FLOOR}
      aria-valuemax={0}
      aria-valuenow={panel.level ?? FLOOR}
    >
      <!-- 緑→黄→赤のグラデーションを、レベルの位置まで見せる -->
      <div
        class="absolute inset-y-0 left-0 bg-linear-to-r from-emerald-500 via-emerald-400 via-70% to-red-500 transition-[width] duration-75"
        style:width="{ratio * 100}%"
        style:background-size="{ratio > 0 ? 100 / ratio : 100}% 100%"
      ></div>
    </div>
    <span class="w-16 text-right font-mono text-xs text-muted-foreground tabular-nums">
      {panel.level === undefined ? "—" : `${Math.round(panel.level)} dB`}
    </span>
  </div>
  <div class="relative mr-19 h-3 text-[10px] text-muted-foreground tabular-nums">
    {#each ticks as t (t)}
      <span class="absolute -translate-x-1/2" style:left="{((t - FLOOR) / -FLOOR) * 100}%">{t}</span
      >
    {/each}
  </div>
</div>
