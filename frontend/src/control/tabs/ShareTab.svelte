<script lang="ts">
  // おすそ分け（DESIGN.md 17.1）。ギフトコードを作って、知り合いに使ってもらう。
  // 自分のキーをサーバーに預けるのは、初めてギフトコードを作るときに確かめてから
  import { Badge } from "$lib/components/ui/badge";
  import { Button } from "$lib/components/ui/button";
  import * as Card from "$lib/components/ui/card";
  import * as Field from "$lib/components/ui/field";
  import { Input } from "$lib/components/ui/input";
  import type { SharedCode } from "../../generated/SharedCode";
  import { api } from "../../lib/api";
  import ConfirmDialog from "../parts/ConfirmDialog.svelte";
  import ExpiryPicker from "../parts/ExpiryPicker.svelte";
  import MaskedCode from "../parts/MaskedCode.svelte";
  import SettingsDialog from "../parts/SettingsDialog.svelte";
  import { dateTime, panel } from "../panel.svelte";
  import { usd } from "../usage";

  const ownKey = $derived(panel.view?.keys.openai.state === "saved");
  const share = $derived(panel.share);

  // 新しいコード。期限の既定は 3 日後
  const threeDays = () => Date.now() + 3 * 24 * 60 * 60 * 1000;
  let memo = $state("");
  let expiresAt = $state<number | null>(threeDays());
  const past = $derived(expiresAt !== null && expiresAt <= panel.now);

  let creating = $state(false);

  async function create() {
    creating = true;
    const body = { memo: memo.trim(), expiresAt };
    const created = await panel.shareAction(() => api.createCode(body), "ギフトコードを作りました");
    creating = false;
    if (created) {
      memo = "";
      expiresAt = threeDays();
    }
    return created;
  }

  /** 預けてから作る。分けておくと、作るのに失敗しても預けた状態が画面に出て、やめられる */
  const consent = async () => (await panel.shareAction(api.enableShare)) && create();

  let consenting = $state(false);
  let stopping = $state(false);

  // 期限を変えるダイアログ
  let editingMemo = $state("");
  let editing = $state<number | null>(null);

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
        自分の OpenAI の API
        キーで、知り合いにデドロ字幕を試してもらえます。ギフトコードを渡すと、その人はキーなしで字幕を出せます。使われた分は自分のキーに請求されます
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
      {:else if !share.enabled && !ownKey}
        <p class="text-sm text-muted-foreground">
          おすそ分けするには、先に「API キー」で自分の OpenAI のキーを保存してください
        </p>
      {:else}
        <form
          class="flex flex-col gap-4"
          onsubmit={(e) => {
            e.preventDefault();
            if (share.enabled) void create();
            else consenting = true;
          }}
        >
          <Field.Field>
            <Field.Label for="share-memo">メモ（自分だけに見えます）</Field.Label>
            <Input
              id="share-memo"
              bind:value={memo}
              maxlength={100}
              placeholder="例: Discord の配信仲間"
            />
          </Field.Field>
          <Field.Field>
            <Field.Label>期限</Field.Label>
            <ExpiryPicker bind:value={expiresAt} />
            <Field.Description>
              {#if past}
                <span class="text-destructive">この期限はもう過ぎています。選び直してください</span>
              {:else}
                {expiresAt === null
                  ? "期限なしで使えます。あとから変えられます"
                  : `${dateTime(expiresAt)} まで使えます。あとから変えられます。期限が過ぎても、出している字幕は止まりません`}
              {/if}
            </Field.Description>
          </Field.Field>
          <Button type="submit" class="w-fit" disabled={past || creating}>ギフトコードを作る</Button
          >
        </form>
      {/if}
    </Card.Content>
  </Card.Root>

  {#if share?.enabled}
    <Card.Root>
      <Card.Header>
        <Card.Title>作ったギフトコード</Card.Title>
        <Card.Description>同じコードを何人に渡しても使えます</Card.Description>
      </Card.Header>
      <Card.Content class="flex flex-col gap-4">
        {#each share.codes as c (c.id)}
          <div class="flex flex-col gap-2 rounded-lg border p-4">
            <div class="flex flex-wrap items-center gap-2">
              <span class="font-medium">{c.memo || "メモなし"}</span>
              {#if c.revoked}<Badge variant="destructive">止めています</Badge>{/if}
              <span class="ml-auto text-sm text-muted-foreground tabular-nums">
                今月 {usd(c.monthUsd)} ／ 合計 {usd(c.usd)}
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
                  trigger="メモを変える"
                  title="メモを変える"
                  valid={true}
                  onopen={() => (editingMemo = c.memo)}
                  onsave={() =>
                    panel.shareAction(
                      () => api.updateCode(c.id, { memo: editingMemo.trim() }),
                      "メモを変えました",
                    )}
                >
                  <Field.Field>
                    <Field.Label for="edit-memo-{c.id}">メモ（自分だけに見えます）</Field.Label>
                    <Input id="edit-memo-{c.id}" maxlength={100} bind:value={editingMemo} />
                  </Field.Field>
                </SettingsDialog>
                <SettingsDialog
                  trigger="期限を変える"
                  title="期限を変える"
                  description={c.memo || undefined}
                  valid={editing === null || Number.isFinite(editing)}
                  onopen={() => (editing = c.expiresAt)}
                  onsave={() =>
                    panel.shareAction(
                      () => api.updateCode(c.id, { expiresAt: editing }),
                      "期限を変えました",
                    )}
                >
                  <ExpiryPicker bind:value={editing} />
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
  bind:open={consenting}
  title="あなたの API キーをサーバーに預けます"
  action="預けてギフトコードを作る"
  disabled={past}
  onconfirm={consent}
>
  <p>
    ギフトコードをもらった人は、自分の API キーを持っていません。そこで、あなたの API
    キーをデドロ字幕のサーバーに預けて、その人の字幕に使います。
  </p>
  <ul class="list-disc pl-5">
    <li>料金: その人が使った分は、あなたの OpenAI に請求されます</li>
    <li>保存: キーは暗号化して保存します。その人の声や字幕は保存しません</li>
    <li>やめるとき: 「おすそ分けをやめる」で、いつでもキーを消せます</li>
  </ul>
  <p>別の PC でも同じキーを預けている場合、その PC では管理できなくなります</p>
</ConfirmDialog>

<ConfirmDialog
  bind:open={stopping}
  title="おすそ分けをやめますか？"
  action="おすそ分けをやめる"
  destructive
  onconfirm={() => panel.shareAction(api.disableShare, "おすそ分けをやめました")}
>
  <p>預けたキーと、作ったギフトコードがすべて消えます。配ったギフトコードは使えなくなります。</p>
</ConfirmDialog>
