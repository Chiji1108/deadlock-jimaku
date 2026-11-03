<script lang="ts">
  import { Badge } from "$lib/components/ui/badge";
  import { Button } from "$lib/components/ui/button";
  import * as Card from "$lib/components/ui/card";
  import * as Field from "$lib/components/ui/field";
  import { Input } from "$lib/components/ui/input";
  import type { Provider } from "../../generated/Provider";
  import OpenAiKeyGuide from "../parts/OpenAiKeyGuide.svelte";
  import { panel } from "../panel.svelte";

  const providers: { provider: Provider; name: string; use: string; url: string }[] = [
    {
      provider: "openai",
      name: "OpenAI",
      use: "文字起こし・翻訳・漢字の名前の読みに使います（必須）",
      url: "https://platform.openai.com/api-keys",
    },
  ];

  let inputs = $state<Record<Provider, string>>({ openai: "" });

  async function save(provider: Provider, key: string) {
    // 失敗したら貼り直さずにやり直せるよう、入力は残す
    if (await panel.setKey(provider, key)) inputs[provider] = "";
  }
</script>

<div class="flex flex-col gap-6">
  <OpenAiKeyGuide open={panel.view?.keys.openai.state !== "saved"} />

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
                {p.use}。<a class="underline" href={p.url} target="_blank" rel="noreferrer"
                  >キーを作る</a
                >
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
</div>
