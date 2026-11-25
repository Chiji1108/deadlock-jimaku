<script lang="ts">
  // 統計の「時間帯」。直近 90 日に字幕を出していた時間を、曜日（行）と時（列）のマスで、長いほど濃くする
  import { HEAT, heat, hoursAndMinutes } from "./stats";

  /** 月曜 0 時から 1 時間ごとに 7 × 24 個（ms） */
  let { hours }: { hours: number[] } = $props();

  const WEEKDAYS = ["月", "火", "水", "木", "金", "土", "日"];
  const max = $derived(Math.max(0, ...hours));
</script>

<div class="flex flex-col gap-2">
  <div class="grid grid-cols-[auto_repeat(24,minmax(0,1fr))] gap-[3px] text-[10px]">
    <div></div>
    {#each Array.from({ length: 24 }, (_, h) => h) as h (h)}
      <div class="leading-none text-muted-foreground">{h % 3 === 0 ? `${h}時` : ""}</div>
    {/each}
    {#each WEEKDAYS as weekday, d (weekday)}
      <div class="pr-1 leading-none text-muted-foreground">{weekday}</div>
      {#each Array.from({ length: 24 }, (_, h) => h) as h (h)}
        {@const ms = hours[d * 24 + h] ?? 0}
        <div
          class="aspect-square rounded-[2px] {heat(ms, max)}"
          title="{weekday} {h} 時台 {hoursAndMinutes(ms)}"
        ></div>
      {/each}
    {/each}
  </div>
  <div class="flex items-center justify-end gap-1 text-[10px] text-muted-foreground">
    少ない
    {#each HEAT as level (level)}<div class="size-[10px] rounded-[2px] {level}"></div>{/each}
    多い
  </div>
</div>
