<script lang="ts">
  // ゲームの画面をどこから読むか（DESIGN.md 18.8）。この PC の Deadlock のウィンドウ（Windows だけ）か、OBS のソース。
  // ゲーム用と配信用の PC を分けている人は、ゲームの画面がキャプチャーボードから OBS に入っているので、OBS からもらう
  import RefreshIcon from "@lucide/svelte/icons/refresh-cw";
  import TrashIcon from "@lucide/svelte/icons/trash-2";
  import { Badge } from "$lib/components/ui/badge";
  import { Button } from "$lib/components/ui/button";
  import * as Card from "$lib/components/ui/card";
  import * as Field from "$lib/components/ui/field";
  import { Input } from "$lib/components/ui/input";
  import * as Select from "$lib/components/ui/select";
  import * as ToggleGroup from "$lib/components/ui/toggle-group";
  import type { ScreenStatus } from "../../generated/ScreenStatus";
  import { api } from "../../lib/api";
  import { obsProblem } from "../../lib/screen-status";
  import { panel } from "../panel.svelte";
  import Help from "./Help.svelte";

  const c = $derived(panel.settings!.purchaseCards);
  /** この PC の Deadlock のウィンドウを読める（Windows）。読めない OS では、いつも OBS から読む */
  const readsWindow = $derived(panel.view!.readsWindow);
  const fromObs = $derived(c.fromObs || !readsWindow);
  const saved = $derived(panel.view!.keys.obs.state === "saved");

  let password = $state("");
  let inputs = $state<string[]>([]);
  /** ソースの一覧を取れない理由 */
  let problem = $state<ScreenStatus>();
  /** 一覧にないもの（OBS で名前を変えた、消した）も、選んだままなら見せる */
  const choices = $derived(
    c.obsInput && !inputs.includes(c.obsInput) ? [c.obsInput, ...inputs] : inputs,
  );

  async function load() {
    const listed = await api.obsInputs();
    [inputs, problem] = Array.isArray(listed) ? [listed, undefined] : [[], listed];
  }

  // OBS から読むことにしたときと、ポートを変えたときに、ソースの一覧を取り直す（ほかの設定を変えたときは取り直さない）
  const port = $derived(c.obsPort);
  $effect(() => {
    if (fromObs && port) void load();
  });

  async function savePassword(value: string) {
    // 失敗したら入れ直さずにやり直せるよう、入力は残す
    if (await panel.setObsPassword(value)) password = "";
    await load();
  }
</script>

<Card.Root>
  <Card.Header>
    <Card.Title class="flex items-center gap-1.5"
      >読む画面<Help
        >ゲーム用と配信用の PC を分けているときは、ゲームの画面が映っている OBS
        のソース（映像キャプチャデバイスなど）から読みます。画面は、同じ PC の OBS
        からもらうだけで、保存も送信もしません。Linux で遊んでいて、ゲームを OBS
        に映しているときも使えます</Help
      ></Card.Title
    >
    <!-- 「この OS では」ではなく、誰に向けた設定かを書く。OBS から読むのは、ゲーム用と配信用の PC を分けている人 -->
    <Card.Description>
      {#if readsWindow}
        ゲーム用と配信用の PC を分けている 2 PC 配信の方は、「OBS
        のソース」を選んでください。キャプチャーボードから OBS に入っているゲームの画面を読みます
      {:else}
        ゲーム用と配信用の PC を分けている 2 PC 配信の方へ。キャプチャーボードから OBS
        に入っているゲームの画面を読みます。下の手順で設定してください
      {/if}
    </Card.Description>
  </Card.Header>
  <Card.Content>
    <Field.Group>
      {#if readsWindow}
        <Field.Field>
          <ToggleGroup.Root
            type="single"
            variant="outline"
            class="w-full"
            value={c.fromObs ? "obs" : "window"}
            onValueChange={(v) => v && panel.update({ purchaseCards: { fromObs: v === "obs" } })}
          >
            <ToggleGroup.Item value="window" class="flex-1">この PC の Deadlock</ToggleGroup.Item>
            <ToggleGroup.Item value="obs" class="flex-1">OBS のソース（2 PC 配信）</ToggleGroup.Item
            >
          </ToggleGroup.Root>
        </Field.Field>
      {/if}
      {#if fromObs}
        <ol class="list-decimal pl-5 text-sm text-muted-foreground">
          <li>
            OBS の「ツール」→「WebSocket サーバー設定」で、「WebSocket
            サーバーを有効にする」をオンにします
          </li>
          <li>同じ画面の「接続情報を表示」に出るパスワードを、下に入れます</li>
          <li>ゲームの画面が映っているソースを選びます</li>
        </ol>
        <form
          onsubmit={(e) => {
            e.preventDefault();
            void savePassword(password);
          }}
        >
          <Field.Field>
            <div class="flex items-center gap-2">
              <Field.Label for="obs-password">OBS のパスワード</Field.Label>
              {#if saved}
                <Badge variant="secondary">保存済み</Badge>
              {:else}
                <Badge variant="outline">未設定</Badge>
              {/if}
            </div>
            <Field.Description>
              OS の資格情報ストアに保存します。OBS で認証をオフにしているときは、要りません
            </Field.Description>
            <div class="flex gap-2">
              <Input
                id="obs-password"
                type="password"
                autocomplete="off"
                bind:value={password}
                placeholder={saved
                  ? "変えるときは新しいパスワードを貼り付け"
                  : "パスワードを貼り付け"}
              />
              <Button type="submit" disabled={!password}>保存</Button>
              {#if saved}
                <Button type="button" variant="outline" onclick={() => savePassword("")}
                  ><TrashIcon />削除</Button
                >
              {/if}
            </div>
          </Field.Field>
        </form>
        <Field.Field>
          <Field.Label for="obs-port"
            >ポート<Help
              >OBS の「WebSocket サーバー設定」に出ている番号です。ふつうは 4455 のままです</Help
            ></Field.Label
          >
          <Input
            id="obs-port"
            type="number"
            min="1"
            max="65535"
            class="w-28"
            value={c.obsPort}
            onchange={(e) => {
              const port = Number(e.currentTarget.value);
              if (Number.isInteger(port) && port >= 1 && port <= 65535)
                void panel.update({ purchaseCards: { obsPort: port } });
              else e.currentTarget.value = String(c.obsPort);
            }}
          />
        </Field.Field>
        <Field.Field>
          <Field.Label>ゲームの画面のソース</Field.Label>
          <div class="flex gap-2">
            <Select.Root
              type="single"
              value={c.obsInput}
              onValueChange={(v) => panel.update({ purchaseCards: { obsInput: v } })}
            >
              <Select.Trigger class="w-full" disabled={choices.length === 0}>
                {c.obsInput || "選んでください"}
              </Select.Trigger>
              <Select.Content>
                {#each choices as name (name)}
                  <Select.Item value={name}>{name}</Select.Item>
                {/each}
              </Select.Content>
            </Select.Root>
            <Button type="button" variant="outline" onclick={load}
              ><RefreshIcon />読み込み直す</Button
            >
          </div>
          {#if problem}
            <Field.Description class="text-destructive">{obsProblem(problem)}</Field.Description>
          {/if}
        </Field.Field>
      {/if}
    </Field.Group>
  </Card.Content>
</Card.Root>
