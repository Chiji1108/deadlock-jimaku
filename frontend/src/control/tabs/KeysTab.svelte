<script lang="ts">
  import { Badge } from "$lib/components/ui/badge";
  import { Button } from "$lib/components/ui/button";
  import * as Card from "$lib/components/ui/card";
  import * as Field from "$lib/components/ui/field";
  import { Input } from "$lib/components/ui/input";
  import type { Provider } from "../../generated/Provider";
  import OpenAiKeyGuide from "../parts/OpenAiKeyGuide.svelte";
  import { giftStatusText, giftValid, panel } from "../panel.svelte";

  const providers: { provider: Provider; name: string; use: string }[] = [
    {
      provider: "openai",
      name: "OpenAI",
      use: "ライト・スタンダード・OpenAI のモードの文字起こし・翻訳・ふりがなに使います。作り方は上にあります",
    },
    {
      provider: "xai",
      name: "xAI",
      use: "ライトのモードの確定の文字起こし（Grok Voice Transcribe）に使います。xAI の API コンソール（console.x.ai）で作れます。ギフトコードでも、おすそ分けした人が xAI のキーを預けていれば使えます",
    },
    {
      provider: "qwen",
      name: "Qwen",
      use: "翻訳を Qwen-MT-Flash にするときに使います。QwenCloud のシンガポール（国際）の API キーです（ほかの地域のキーは使えません）。ギフトコードでは使えません",
    },
  ];

  let inputs = $state<Record<Provider, string>>({ openai: "", xai: "", qwen: "" });

  async function save(provider: Provider, key: string) {
    // 失敗したら貼り直さずにやり直せるよう、入力は残す
    if (await panel.setKey(provider, key)) inputs[provider] = "";
  }

  let gift = $state("");
  const giftSaved = $derived(panel.view?.keys.gift.state === "saved");
</script>

<div class="flex flex-col gap-6">
  <OpenAiKeyGuide open={panel.keyMissing} />

  <Card.Root>
    <Card.Header>
      <Card.Title>API キー</Card.Title>
      <Card.Description>
        OS の資格情報ストア（Windows の資格情報マネージャー、Mac のキーチェーン、Linux の Secret
        Service）に保存します。保存したキーは画面に表示しません
      </Card.Description>
    </Card.Header>
    <Card.Content>
      <Field.Group>
        {#each providers as p, i (p.provider)}
          {@const key = panel.view?.keys[p.provider]}
          {@const saved = key?.state === "saved"}
          {#if i > 0}<Field.Separator />{/if}
          <form
            onsubmit={(e) => {
              e.preventDefault();
              void save(p.provider, inputs[p.provider]);
            }}
          >
            <Field.Field>
              <div class="flex items-center gap-2">
                <Field.Label for={p.provider}>{p.name}</Field.Label>
                {#if saved}
                  <Badge variant="secondary">保存済み</Badge>
                {:else if key?.state === "unreadable"}
                  <Badge variant="destructive">読めません</Badge>
                {:else}
                  <Badge variant="outline">未設定</Badge>
                {/if}
              </div>
              <Field.Description>
                {p.use}
              </Field.Description>
              {#if key?.state === "unreadable"}
                <Field.Description class="text-destructive">
                  OS の資格情報ストアを読めません（{key.reason}）。アクセスを許可するか、ロックを解除してください
                </Field.Description>
              {/if}
              <div class="flex gap-2">
                <Input
                  id={p.provider}
                  type="password"
                  autocomplete="off"
                  bind:value={inputs[p.provider]}
                  placeholder={saved ? "変えるときは新しいキーを貼り付け" : "キーを貼り付け"}
                />
                <Button type="submit" disabled={!inputs[p.provider]}>保存</Button>
                {#if saved}
                  <Button type="button" variant="outline" onclick={() => save(p.provider, "")}>
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
        キーなしで使えます。使った分はおすそ分けしてくれた人に請求されます
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
