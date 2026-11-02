<script lang="ts">
  import { Badge } from "$lib/components/ui/badge";
  import { Button } from "$lib/components/ui/button";
  import * as Card from "$lib/components/ui/card";
  import * as Field from "$lib/components/ui/field";
  import { Input } from "$lib/components/ui/input";
  import type { KeySource } from "../../generated/KeySource";
  import type { Provider } from "../../generated/Provider";
  import { panel } from "../panel.svelte";

  const providers: { provider: Provider; name: string; use: string; url: string }[] = [
    {
      provider: "mistral",
      name: "Mistral",
      use: "文字起こしに使います（必須）",
      url: "https://console.mistral.ai/api-keys",
    },
    {
      provider: "openai",
      name: "OpenAI",
      use: "翻訳と、漢字の名前の読みに使います",
      url: "https://platform.openai.com/api-keys",
    },
  ];
  const sourceLabel: Record<KeySource, string> = { keyring: "保存済み", env: "環境変数" };

  let inputs = $state<Record<Provider, string>>({ mistral: "", openai: "" });

  async function save(provider: Provider, key: string) {
    // 失敗したら貼り直さずにやり直せるよう、入力は残す
    if (await panel.setKey(provider, key)) inputs[provider] = "";
  }
</script>

<Card.Root>
  <Card.Header>
    <Card.Title>API キー</Card.Title>
    <Card.Description>
      OS の資格情報ストア（Windows の資格情報マネージャー、Mac のキーチェーン）に保存します。
      保存したキーは画面に表示しません
    </Card.Description>
  </Card.Header>
  <Card.Content>
    <Field.Group>
      {#each providers as p, i (p.provider)}
        {@const source = panel.view?.keys[p.provider]}
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
              {#if source}
                <Badge variant="secondary">{sourceLabel[source]}</Badge>
              {:else}
                <Badge variant="outline">未設定</Badge>
              {/if}
            </div>
            <Field.Description>
              {p.use}。<a class="underline" href={p.url} target="_blank" rel="noreferrer"
                >キーを作る</a
              >
            </Field.Description>
            <div class="flex gap-2">
              <Input
                id={p.provider}
                type="password"
                autocomplete="off"
                bind:value={inputs[p.provider]}
                disabled={source === "env"}
                placeholder={source === "env"
                  ? "環境変数のキーを使っています"
                  : source
                    ? "変えるときは新しいキーを貼り付け"
                    : "キーを貼り付け"}
              />
              <Button type="submit" disabled={!inputs[p.provider]}>保存</Button>
              {#if source === "keyring"}
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
