<script lang="ts">
  import { Badge } from "$lib/components/ui/badge";
  import { Button } from "$lib/components/ui/button";
  import * as Card from "$lib/components/ui/card";
  import * as Field from "$lib/components/ui/field";
  import { Input } from "$lib/components/ui/input";
  import { PROVIDERS } from "../../generated/catalog";
  import type { Provider } from "../../generated/Provider";
  import { giftStatusText, giftValid, panel } from "../panel.svelte";

  /** 入れる前に知らないと、保存しても使えないことだけを書く。使い道は「モード」のタブに出ている */
  const NOTES: Partial<Record<Provider, string>> = {
    qwen: "QwenCloud のシンガポール（国際）のキーだけが使えます",
  };

  let inputs = $state(
    Object.fromEntries(PROVIDERS.map((p) => [p.id, ""])) as Record<Provider, string>,
  );

  async function save(provider: Provider, key: string) {
    // 失敗したら貼り直さずにやり直せるよう、入力は残す
    if (await panel.setKey(provider, key)) inputs[provider] = "";
  }

  let gift = $state("");
  const giftSaved = $derived(panel.view?.keys.gift.state === "saved");
</script>

<div class="flex flex-col gap-6">
  <Card.Root>
    <Card.Header>
      <Card.Title>API キー</Card.Title>
      <Card.Description>
        工程に有料のモデルを選んだときに使います（どの工程にどのキーが要るかは、「モード」のタブに出ます）。OS
        の資格情報ストアに保存し、保存したキーは画面に表示しません
      </Card.Description>
    </Card.Header>
    <Card.Content>
      <Field.Group>
        {#each PROVIDERS as p, i (p.id)}
          {@const key = panel.view?.keys[p.id]}
          {@const note = NOTES[p.id]}
          {@const saved = key?.state === "saved"}
          {#if i > 0}<Field.Separator />{/if}
          <form
            onsubmit={(e) => {
              e.preventDefault();
              void save(p.id, inputs[p.id]);
            }}
          >
            <Field.Field>
              <div class="flex items-center gap-2">
                <Field.Label for={p.id}>{p.name}</Field.Label>
                {#if saved}
                  <Badge variant="secondary">保存済み</Badge>
                {:else if key?.state === "unreadable"}
                  <Badge variant="destructive">読めません</Badge>
                {:else}
                  <Badge variant="outline">未設定</Badge>
                {/if}
              </div>
              {#if note}<Field.Description>{note}</Field.Description>{/if}
              {#if key?.state === "unreadable"}
                <Field.Description class="text-destructive">
                  OS の資格情報ストアを読めません（{key.reason}）。アクセスを許可するか、ロックを解除してください
                </Field.Description>
              {/if}
              <div class="flex gap-2">
                <Input
                  id={p.id}
                  type="password"
                  autocomplete="off"
                  bind:value={inputs[p.id]}
                  placeholder={saved ? "変えるときは新しいキーを貼り付け" : "キーを貼り付け"}
                />
                <Button type="submit" disabled={!inputs[p.id]}>保存</Button>
                {#if saved}
                  <Button type="button" variant="outline" onclick={() => save(p.id, "")}>
                    削除
                  </Button>
                {/if}
              </div>
            </Field.Field>
          </form>
        {/each}
      </Field.Group>
    </Card.Content>
  </Card.Root>

  <Card.Root>
    <Card.Header>
      <Card.Title>ギフトコード</Card.Title>
      <Card.Description>
        デドロ字幕を使っている人から「おすそ分け」のコードをもらったら、ここに入れると自分の API
        キーなしで使えます。使った分はおすそ分けしてくれた人に請求されます。おすそ分けは 0.7
        でなくなります（キーなしで使うなら、「おすすめ」のモードを選んでください）
      </Card.Description>
    </Card.Header>
    <Card.Content>
      <form
        onsubmit={async (e) => {
          e.preventDefault();
          if (await panel.setGift(gift)) gift = "";
        }}
      >
        <Field.Field>
          <div class="flex items-center gap-2">
            <Field.Label for="gift">コード</Field.Label>
            {#if giftSaved}
              <Badge variant={giftValid(panel.gift) ? "secondary" : "destructive"}>
                {giftValid(panel.gift) ? "保存済み" : "使えません"}
              </Badge>
            {:else}
              <Badge variant="outline">未設定</Badge>
            {/if}
          </div>
          {#if giftSaved && panel.gift}
            <Field.Description class={giftValid(panel.gift) ? "" : "text-destructive"}>
              {giftStatusText(panel.gift)}
            </Field.Description>
          {/if}
          {#if giftSaved && (panel.view?.keys.openai.state === "saved" || panel.view?.keys.xai.state === "saved")}
            <Field.Description
              >自分の API キーを保存している会社には、自分のキーを使います</Field.Description
            >
          {/if}
          <div class="flex gap-2">
            <Input
              id="gift"
              type="password"
              autocomplete="off"
              bind:value={gift}
              placeholder={giftSaved ? "変えるときは新しいコードを入力" : "XXXX-XXXX-XXXX"}
            />
            <Button type="submit" disabled={!gift.trim()}>保存</Button>
            {#if giftSaved}
              <Button type="button" variant="outline" onclick={panel.removeGift}>削除</Button>
            {/if}
          </div>
        </Field.Field>
      </form>
    </Card.Content>
  </Card.Root>
</div>
