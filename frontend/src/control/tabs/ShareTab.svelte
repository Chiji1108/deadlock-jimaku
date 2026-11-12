<script lang="ts">
  // おすそ分け（DESIGN.md 17.1）。自分のキーを預けてギフトコードを作り、知り合いに使ってもらう。
  // キーは会社ごとに、確かめてから預ける。コードはどれも、預けたキーの会社に使える
  import { Badge } from "$lib/components/ui/badge";
  import { Button } from "$lib/components/ui/button";
  import * as Card from "$lib/components/ui/card";
  import type { Provider } from "../../generated/Provider";
  import type { SharedCode } from "../../generated/SharedCode";
  import { api } from "../../lib/api";
  import { PROVIDER_NAMES } from "../../lib/engines";
  import ConfirmDialog from "../parts/ConfirmDialog.svelte";
  import CodeFields, { validCode } from "../parts/CodeFields.svelte";
  import { DEFAULT_LIMIT, validLimit } from "../parts/DailyLimitField.svelte";
  import MaskedCode from "../parts/MaskedCode.svelte";
  import SettingsDialog from "../parts/SettingsDialog.svelte";
  import { dateTime, panel } from "../panel.svelte";
  import { usd } from "../usage";

  /** 預けられる会社と、ギフトコードの人が使えるもの・料金（預ける前の確認に出す） */
  const PROVIDERS: { provider: Provider; uses: string; price: string }[] = [
    {
      provider: "openai",
      uses: "途中の字幕（GPT-Live-Transcribe）・確定の文字起こし（GPT-Transcribe）・翻訳と用語・ふりがな",
      price: "使い方で変わります（文字起こしと翻訳をすべて GPT にして、1 時間 約 $0.6）",
    },
    {
      provider: "xai",
      uses: "確定の文字起こし（Grok Voice Transcribe）",
      price: "1 時間 $0.10",
    },
  ];

  const share = $derived(panel.share);
  const shared = $derived(share?.enabled ? share.providers : []);
  const saved = (p: Provider) => panel.view?.keys[p].state === "saved";
  const anySaved = $derived(PROVIDERS.some(({ provider }) => saved(provider)));

  // 新しいコード。期限の既定は 3 日後
  const threeDays = () => Date.now() + 3 * 24 * 60 * 60 * 1000;
  let memo = $state("");
  let expiresAt = $state<number | null>(threeDays());
  let dailyLimit = $state<number | null>(DEFAULT_LIMIT);
  const valid = $derived(validCode(expiresAt, dailyLimit, panel.now));

  let creating = $state(false);

  async function create() {
    if (!validLimit(dailyLimit)) return false;
    creating = true;
    const body = { memo: memo.trim(), expiresAt, dailyLimitUsd: dailyLimit };
    const created = await panel.shareAction(() => api.createCode(body), "ギフトコードを作りました");
    creating = false;
    if (created) {
      memo = "";
      expiresAt = threeDays();
      dailyLimit = DEFAULT_LIMIT;
    }
    return created;
  }

  /** 預ける前に確かめる会社 */
  let consenting = $state<Provider | undefined>();
  let consentOpen = $state(false);
  const consent = $derived(PROVIDERS.find((p) => p.provider === consenting));
  let stopping = $state(false);

  async function deposit() {
    if (!consent) return false;
    const { provider } = consent;
    return panel.shareAction(
      () => api.shareKey(provider),
      `${PROVIDER_NAMES[provider]} のキーを預けました`,
    );
  }

  /** 設定を変えるダイアログの中身 */
  let editing = $state({
    memo: "",
    expiresAt: null as number | null,
    dailyLimit: null as number | null,
  });

  async function saveEdit(id: string) {
    const limit = editing.dailyLimit;
    if (!validLimit(limit)) return false;
    const body = { memo: editing.memo.trim(), expiresAt: editing.expiresAt, dailyLimitUsd: limit };
    return panel.shareAction(() => api.updateCode(id, body), "設定を変えました");
  }

  function expiry(c: SharedCode) {
    if (c.expiresAt === null) return "期限なし";
    return c.expiresAt <= panel.now
      ? `期限切れ（${dateTime(c.expiresAt)}）`
      : `${dateTime(c.expiresAt)} まで`;
  }
</script>

<div class="flex flex-col gap-6">
  <Card.Root>
    <Card.Header>
      <Card.Title>おすそ分け</Card.Title>
      <Card.Description>
        自分の API
        キーを預けて、知り合いにデドロ字幕を使ってもらえます。ギフトコードを渡すと、その人はキーなしで字幕を出せます。使われた分は、預けたキーの会社から自分に請求されます
      </Card.Description>
    </Card.Header>
    <Card.Content class="flex flex-col gap-4">
      {#if !share}
        <p class="text-sm text-muted-foreground">
          {panel.shareError
            ? `おすそ分けの状態を取れません（${panel.shareError}）`
            : "読み込んでいます…"}
        </p>
        {#if panel.shareError}
          <Button variant="outline" class="w-fit" onclick={() => (stopping = true)}>
            おすそ分けをやめる
          </Button>
        {/if}
      {:else}
        {#each PROVIDERS as p (p.provider)}
          {@const on = shared.includes(p.provider)}
          <div class="flex flex-col gap-2 rounded-lg border p-4">
            <div class="flex flex-wrap items-center gap-2">
              <span class="font-medium">{PROVIDER_NAMES[p.provider]}</span>
              {#if on}<Badge variant="secondary">預けています</Badge>{/if}
            </div>
            {#if on}
              <Button
                variant="outline"
                class="w-fit"
                disabled={shared.length === 1}
                onclick={() =>
                  panel.shareAction(
                    () => api.unshareKey(p.provider),
                    `${PROVIDER_NAMES[p.provider]} のキーを消しました`,
                  )}>預けるのをやめる</Button
              >
              {#if shared.length === 1}
                <p class="text-xs text-muted-foreground">
                  最後のキーです。やめるときは下の「おすそ分けをやめる」を押してください
                </p>
              {/if}
            {:else if saved(p.provider)}
              <Button
                class="w-fit"
                onclick={() => {
                  consenting = p.provider;
                  consentOpen = true;
                }}>預ける</Button
              >
            {:else}
              <p class="text-xs text-muted-foreground">
                預けるには、先に「API キー」で自分の {PROVIDER_NAMES[p.provider]} のキーを保存してください
              </p>
            {/if}
          </div>
        {/each}
        {#if !share.enabled && anySaved}
          <Button variant="outline" class="w-fit" onclick={() => panel.shareAction(api.joinShare)}
            >ほかの PC のおすそ分けを表示する</Button
          >
        {/if}
      {/if}
    </Card.Content>
  </Card.Root>

  {#if share?.enabled}
    <Card.Root>
      <Card.Header>
        <Card.Title>ギフトコード</Card.Title>
        <Card.Description>
          同じコードを何人に渡しても使えます。どのコードも {shared
            .map((p) => PROVIDER_NAMES[p])
            .join(" と ")} に使えます
        </Card.Description>
      </Card.Header>
      <Card.Content class="flex flex-col gap-4">
        <form
          class="flex flex-col gap-4"
          onsubmit={(e) => {
            e.preventDefault();
            void create();
          }}
        >
          <CodeFields
            id="share"
            bind:memo
            bind:expiresAt
            bind:dailyLimit
            placeholder="例: Discord の配信仲間"
          />
          <Button type="submit" class="w-fit" disabled={!valid || creating}
            >ギフトコードを作る</Button
          >
        </form>
        {#each share.codes as c (c.id)}
          <div class="flex flex-col gap-2 rounded-lg border p-4">
            <div class="flex flex-wrap items-center gap-2">
              <span class="font-medium">{c.memo || "メモなし"}</span>
              {#if c.revoked}<Badge variant="destructive">止めています</Badge>{/if}
              <span class="ml-auto text-sm text-muted-foreground tabular-nums">
                今日 {usd(c.dayUsd)}（上限 {usd(c.effectiveLimitUsd)}）／ 今月 {usd(c.monthUsd)} ／ 合計
                {usd(c.usd)}
              </span>
            </div>
            {#if c.code}
              <MaskedCode code={c.code} />
            {:else}
              <p class="text-xs text-muted-foreground">
                ほかの PC で作ったコードです（コードはその PC で見られます）
              </p>
            {/if}
            <div class="flex flex-wrap items-center gap-2 text-sm">
              <span class="text-muted-foreground">{expiry(c)}</span>
              <span class="ml-auto flex gap-1">
                <SettingsDialog
                  trigger="設定を変える"
                  title="設定を変える"
                  description={c.memo || undefined}
                  valid={validCode(editing.expiresAt, editing.dailyLimit, panel.now, c.expiresAt)}
                  onopen={() =>
                    (editing = {
                      memo: c.memo,
                      expiresAt: c.expiresAt,
                      dailyLimit: c.effectiveLimitUsd,
                    })}
                  onsave={() => saveEdit(c.id)}
                >
                  <CodeFields
                    id="edit-{c.id}"
                    bind:memo={editing.memo}
                    bind:expiresAt={editing.expiresAt}
                    bind:dailyLimit={editing.dailyLimit}
                    saved={c.expiresAt}
                  />
                </SettingsDialog>
                <Button
                  variant="ghost"
                  size="sm"
                  onclick={() =>
                    panel.shareAction(() => api.updateCode(c.id, { revoked: !c.revoked }))}
                >
                  {c.revoked ? "使えるように戻す" : "止める"}
                </Button>
              </span>
            </div>
          </div>
        {:else}
          <p class="text-sm text-muted-foreground">まだギフトコードがありません</p>
        {/each}
      </Card.Content>
      <Card.Footer>
        <Button variant="outline" onclick={() => (stopping = true)}>おすそ分けをやめる</Button>
      </Card.Footer>
    </Card.Root>
  {/if}
</div>

<ConfirmDialog
  bind:open={consentOpen}
  title="あなたの {consent ? PROVIDER_NAMES[consent.provider] : ''} の API キーをサーバーに預けます"
  action="預ける"
  onconfirm={deposit}
>
  {#if consent}
    <p>
      ギフトコードをもらった人は、自分の API
      キーを持っていません。そこで、あなたのキーをデドロ字幕のサーバーに預けて、その人の字幕に使います。その人が使えるもの:
      {consent.uses}。
    </p>
    <ul class="list-disc pl-5">
      <li>
        料金: その人が使った分は、あなたの {PROVIDER_NAMES[consent.provider]} に請求されます（{consent.price}。コードごとの
        1 日の上限まで）
      </li>
      <li>保存: キーは暗号化して保存します。その人の声や字幕は保存しません</li>
      <li>やめるとき: 「預けるのをやめる」か「おすそ分けをやめる」で、いつでも消せます</li>
    </ul>
  {/if}
</ConfirmDialog>

<ConfirmDialog
  bind:open={stopping}
  title="おすそ分けをやめますか？"
  action="おすそ分けをやめる"
  destructive
  onconfirm={() => panel.shareAction(api.disableShare, "おすそ分けをやめました")}
>
  <p>
    預けたキーと、作ったギフトコードがすべて消えます。配ったギフトコードは使えなくなります。同じおすそ分けを見ているほかの
    PC でも止まります。
  </p>
</ConfirmDialog>
