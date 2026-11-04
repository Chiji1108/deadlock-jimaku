<script lang="ts">
  // おすそ分けのコードの期限。ボタンで手早く決めるか、日時の欄で細かく決める。null は期限なし
  import { Button } from "$lib/components/ui/button";
  import { Input } from "$lib/components/ui/input";

  let { value = $bindable() }: { value: number | null } = $props();

  const DAY = 24 * 60 * 60 * 1000;
  const PRESETS = [
    { label: "1 日後", days: 1 },
    { label: "3 日後", days: 3 },
    { label: "1 週間後", days: 7 },
  ];

  /** `<input type="datetime-local">` の値（その PC の時刻）とミリ秒の変換 */
  const toLocal = (ms: number) => {
    const d = new Date(ms - new Date(ms).getTimezoneOffset() * 60_000);
    return d.toISOString().slice(0, 16);
  };
  const fromLocal = (s: string) => (s ? new Date(s).getTime() : NaN);
</script>

<div class="flex flex-col gap-2">
  <div class="flex flex-wrap gap-2">
    {#each PRESETS as p (p.days)}
      <Button variant="outline" size="sm" onclick={() => (value = Date.now() + p.days * DAY)}>
        {p.label}
      </Button>
    {/each}
    <Button
      variant={value === null ? "default" : "outline"}
      size="sm"
      onclick={() => (value = null)}
    >
      期限なし
    </Button>
  </div>
  {#if value !== null}
    <Input
      type="datetime-local"
      aria-label="期限"
      class="w-fit"
      value={toLocal(value)}
      onchange={(e) => {
        const ms = fromLocal(e.currentTarget.value);
        if (Number.isFinite(ms)) value = ms;
      }}
    />
  {/if}
</div>
