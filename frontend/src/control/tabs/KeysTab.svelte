<script lang="ts">
  import ChevronIcon from "@lucide/svelte/icons/chevron-down";
  import { Badge } from "$lib/components/ui/badge";
  import { Button } from "$lib/components/ui/button";
  import * as Card from "$lib/components/ui/card";
  import * as Field from "$lib/components/ui/field";
  import { Input } from "$lib/components/ui/input";
  import { PROVIDERS } from "../../generated/catalog";
  import type { Provider } from "../../generated/Provider";
  import { providers } from "../../lib/engines";
  import { panel } from "../panel.svelte";
  import UsageTab from "./UsageTab.svelte";

  /** 入れる前に知らないと、保存しても使えないことだけを書く。使い道は「モード」のタブに出ている */
  const NOTES: Partial<Record<Provider, string>> = {
    qwen: "QwenCloud のシンガポール（国際）のキーだけが使えます",
  };

  let inputs = $state(
    Object.fromEntries(PROVIDERS.map((p) => [p.id, ""])) as Record<Provider, string>,
  );

  /** 今のモードが使う会社と、キーを保存してある会社を上に出す。ほかの会社は、開いたときに出す */
  const used = $derived(providers(panel.settings!.engines));
  const shown = (id: Provider) => used.includes(id) || panel.view?.keys[id].state !== "missing";
  const main = $derived(PROVIDERS.filter((p) => shown(p.id)));
  const rest = $derived(PROVIDERS.filter((p) => !shown(p.id)));
  let opened = $state(false);

  async function save(provider: Provider, key: string) {
    // 失敗したら貼り直さずにやり直せるよう、入力は残す
    if (await panel.setKey(provider, key)) inputs[provider] = "";
  }
</script>

{#snippet field(p: (typeof PROVIDERS)[number])}
  {@const key = panel.view?.keys[p.id]}
  {@const note = NOTES[p.id]}
  {@const saved = key?.state === "saved"}
  {@const needed = used.includes(p.id)}
  <form
    onsubmit={(e) => {
      e.preventDefault();
      void save(p.id, inputs[p.id]);
    }}
  >
    <Field.Field>
      <div class="flex items-center gap-2">
        <Field.Label for={p.id}>{p.name}</Field.Label>
        {#if needed}<Badge>今のモードで使う</Badge>{/if}
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
          <Button type="button" variant="outline" onclick={() => save(p.id, "")}>削除</Button>
        {/if}
      </div>
    </Field.Field>
  </form>
{/snippet}

<div class="flex flex-col gap-6">
  <Card.Root>
    <Card.Header>
      <Card.Title>API キー</Card.Title>
      <Card.Description>
        自分の API
        キーで使うモード（「最高精度」や、工程ごとに選んだ有料のモデル）に要ります。「おすすめ」や
        Chrome のモードには要りません。OS の資格情報ストアに保存し、保存したキーは画面に表示しません
      </Card.Description>
    </Card.Header>
    <Card.Content>
      <Field.Group>
        {#if main.length === 0}
          <p class="text-sm text-muted-foreground">今のモードは、API キーを使いません</p>
        {/if}
        {#each main as p, i (p.id)}
          {#if i > 0}<Field.Separator />{/if}
          {@render field(p)}
        {/each}
        {#if rest.length}
          <Field.Separator />
          <Button
            variant="ghost"
            size="sm"
            class="self-start text-muted-foreground"
            aria-expanded={opened}
            onclick={() => (opened = !opened)}
          >
            <ChevronIcon class={["transition-transform", opened && "rotate-180"]} />
            ほかの会社のキー（{rest.map((p) => p.name).join("、")}）
          </Button>
          {#if opened}
            {#each rest as p (p.id)}
              <Field.Separator />
              {@render field(p)}
            {/each}
          {/if}
        {/if}
      </Field.Group>
    </Card.Content>
  </Card.Root>
  <!-- 上の帯の「起動してからの料金」から、ここへ送る（`panel.show`） -->
  <div id="usage" class="scroll-mt-6"><UsageTab /></div>
</div>
