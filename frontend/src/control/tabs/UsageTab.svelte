<script lang="ts">
  import * as Card from "$lib/components/ui/card";
  import type { Use } from "../../generated/Use";
  import { PROVIDER_NAMES } from "../../lib/engines";
  import { panel } from "../panel.svelte";
  import { total, unpriced as hasUnpriced, usd } from "../usage";

  const rows: { use: Use; name: string; audio: boolean }[] = [
    { use: "realtime", name: "話している途中の字幕", audio: true },
    { use: "batch", name: "確定した字幕", audio: true },
    { use: "translate", name: "用語と翻訳", audio: false },
    { use: "furigana", name: "ふりがな", audio: false },
    { use: "readings", name: "辞書の読み", audio: false },
  ];
  const columns = [
    { key: "session", name: "起動してから" },
    { key: "month", name: "今月" },
  ] as const;

  const minutes = (s: number) => (s < 60 ? `${Math.round(s)} 秒` : `${(s / 60).toFixed(1)} 分`);
  const tokens = (n: number) =>
    n < 10_000 ? `${n} トークン` : `${(n / 10_000).toFixed(1)} 万トークン`;

  const unpriced = $derived(columns.some((c) => hasUnpriced(panel.usage?.[c.key])));

  // おすそ分けで使われた分も、預けたキーに請求される（今月の分だけ。コードごとはおすそ分けタブ）
  const shared = $derived(
    panel.share?.enabled ? panel.share.codes.reduce((sum, c) => sum + c.monthUsd, 0) : undefined,
  );
  const giftPaid = $derived(panel.giftPaid.map((p) => PROVIDER_NAMES[p]).join(" と "));
</script>

<Card.Root>
  <Card.Header>
    <Card.Title>料金（概算）</Card.Title>
    <Card.Description>
      送った音声の長さと使ったトークン数に、OpenAI・xAI・Qwen の単価を掛けた目安です。正確な金額は
      <a class="underline" href="https://platform.openai.com/usage" target="_blank" rel="noreferrer"
        >OpenAI</a
      >
      と <a class="underline" href="https://console.x.ai" target="_blank" rel="noreferrer">xAI</a> の使用量のページで確かめてください。今月は
      UTC の月で数えます。応答が届かなかった分（タイムアウトなど）は数えられないので、少なめに出ることがあります
    </Card.Description>
  </Card.Header>
  <Card.Content>
    <div class="grid grid-cols-[1fr_auto_auto] gap-x-6 gap-y-2 text-sm">
      <div></div>
      {#each columns as c (c.key)}
        <div class="text-right text-xs text-muted-foreground">{c.name}</div>
      {/each}
      {#each rows as r (r.use)}
        <div>{r.name}</div>
        {#each columns as c (c.key)}
          {@const a = panel.usage?.[c.key][r.use]}
          <div class="text-right tabular-nums">
            {usd(a?.usd ?? 0)}
            <div class="text-xs text-muted-foreground">
              {r.audio ? minutes(a?.seconds ?? 0) : tokens(a?.tokens ?? 0)}
            </div>
          </div>
        {/each}
      {/each}
      {#if shared !== undefined}
        <div>おすそ分けで使われた分</div>
        <div class="text-right text-muted-foreground">—</div>
        <div class="text-right tabular-nums">{usd(shared)}</div>
      {/if}
      <div class="border-t pt-2 font-medium">合計</div>
      {#each columns as c (c.key)}
        <div class="border-t pt-2 text-right font-medium tabular-nums">
          {usd(total(panel.usage?.[c.key]) + (c.key === "month" ? (shared ?? 0) : 0))}
        </div>
      {/each}
    </div>
    {#if giftPaid}
      <p class="mt-4 text-xs text-muted-foreground">
        ギフトコードで使っているので、{giftPaid} の料金はおすそ分けしてくれた人に請求されます
      </p>
    {/if}
    {#if unpriced}
      <p class="mt-4 text-xs text-muted-foreground">
        単価の分からないモデルを使った分は、合計に入っていません
      </p>
    {/if}
  </Card.Content>
</Card.Root>
