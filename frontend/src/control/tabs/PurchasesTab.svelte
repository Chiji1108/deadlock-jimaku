<script lang="ts">
  // 買ったアイテムのオーバーレイの設定（DESIGN.md 18.5）。ゲームの画面を読むので、Windows でだけ動く。
  // ほかの OS でもタブは出し、使えない理由と見本を見せる（隠すと、機能があることも分からない）
  import Help from "../parts/Help.svelte";
  import MonitorIcon from "@lucide/svelte/icons/monitor";
  import * as Alert from "$lib/components/ui/alert";
  import * as Card from "$lib/components/ui/card";
  import * as Field from "$lib/components/ui/field";
  import * as Select from "$lib/components/ui/select";
  import { Switch } from "$lib/components/ui/switch";
  import * as ToggleGroup from "$lib/components/ui/toggle-group";
  import AddToObs from "../parts/AddToObs.svelte";
  import AnchorPicker from "../parts/AnchorPicker.svelte";
  import OverlayPage from "../parts/OverlayPage.svelte";
  import OverlayPreview from "../parts/OverlayPreview.svelte";
  import { PURCHASES_SIZE } from "../../lib/overlay-size";
  import { overlayUrl, panel } from "../panel.svelte";

  const HOLDS = [4, 6, 8, 12];
  const FOLLOW = "";

  const s = $derived(panel.settings!);
  const c = $derived(s.purchaseCards);
  const usable = $derived(panel.view!.readsScreen);
  // アイテムの名前を取ってくる言語（話す言語・英語・翻訳先）から選ぶ
  const languages = $derived([...new Set([s.micLanguage, "en", ...s.translateTo])]);
  /** 選べる言語にないもの（翻訳先から外した）は、サーバーと同じく話す言語に合わせる */
  const language = $derived(c.language && languages.includes(c.language) ? c.language : FOLLOW);
</script>

<OverlayPage>
  {#snippet preview()}
    <OverlayPreview
      src="/overlay/purchases.html?preview"
      title="買ったアイテムのオーバーレイのプレビュー"
      size={PURCHASES_SIZE}
    />
    {#if usable}
      <AddToObs
        url={overlayUrl("purchases")}
        name="買ったアイテム"
        size={PURCHASES_SIZE}
        hint="大きさは、ソースを引き伸ばさずにプロパティの幅と高さを同じ割合で変えてください"
      />
    {/if}
  {/snippet}

  <div class="flex flex-col gap-6">
    {#if !usable}
      <Alert.Root>
        <MonitorIcon />
        <Alert.Title>Deadlock を動かしている Windows の PC でだけ使えます</Alert.Title>
        <Alert.Description>
          ゲームの画面を読んで、アイテムを買った瞬間に、買ったアイテムを配信に出す機能です。
          このアプリを、ゲームと同じ Windows の PC で動かすと使えます
        </Alert.Description>
      </Alert.Root>
    {:else}
      <Card.Root>
        <Card.Header>
          <Card.Title class="flex items-center gap-1.5"
            >買ったアイテムのオーバーレイ<Help
              >ゲームの画面を PC
              の中だけで見て、持ち物が増えたときに、買ったアイテムを出します。画像は保存も送信もしません。OBS
              と同じ画面の取り込みです</Help
            ></Card.Title
          >
        </Card.Header>
        <Card.Content class="flex flex-col gap-6">
          <Field.Group>
            <Field.Field orientation="horizontal">
              <Field.Content>
                <Field.Label
                  >ゲームの画面を読む<Help
                    >読むのは、このオーバーレイが OBS
                    につながっている間だけです。ショップを開いている間は出さず、閉じたときに、増えた分をまとめて出します</Help
                  ></Field.Label
                >
                <Field.Description>
                  ヒーローやアイテム、HUD の見た目を変える MOD
                  を入れていると読めません。ゲームのアップデートで画面の見た目が変わったときも、読めなくなることがあります。読めないときは何も出ないだけで、字幕やほかのオーバーレイには影響しません
                </Field.Description>
              </Field.Content>
              <Switch
                aria-label="ゲームの画面を読む"
                checked={c.enabled}
                onCheckedChange={(enabled) => panel.update({ purchaseCards: { enabled } })}
              />
            </Field.Field>
            <Field.Field>
              <Field.Label>出しておく時間</Field.Label>
              <ToggleGroup.Root
                type="single"
                variant="outline"
                class="w-full"
                value={String(c.holdSeconds)}
                onValueChange={(v) =>
                  v && panel.update({ purchaseCards: { holdSeconds: Number(v) } })}
              >
                {#each HOLDS as n (n)}
                  <ToggleGroup.Item value={String(n)} class="flex-1">{n} 秒</ToggleGroup.Item>
                {/each}
              </ToggleGroup.Root>
            </Field.Field>
            <Field.Field>
              <Field.Label
                >位置<Help>OBS のソースの箱の中で、表示を寄せる位置です</Help></Field.Label
              >
              <AnchorPicker
                label="買ったアイテムの位置"
                value={c.anchor}
                onchange={(anchor) => panel.update({ purchaseCards: { anchor } })}
              />
            </Field.Field>
            <Field.Field>
              <Field.Label
                >アイテムの名前の言語<Help>統計のラベルは、日本語のほかは英語で出します</Help
                ></Field.Label
              >
              <Select.Root
                type="single"
                value={language}
                onValueChange={(v) => panel.update({ purchaseCards: { language: v } })}
              >
                <Select.Trigger class="w-full">
                  {language === FOLLOW
                    ? `話す言語に合わせる（${panel.label(s.micLanguage)}）`
                    : panel.label(language)}
                </Select.Trigger>
                <Select.Content>
                  <Select.Item value={FOLLOW}
                    >話す言語に合わせる（{panel.label(s.micLanguage)}）</Select.Item
                  >
                  {#each languages as code (code)}
                    <Select.Item value={code}>{panel.label(code)}</Select.Item>
                  {/each}
                </Select.Content>
              </Select.Root>
            </Field.Field>
            <Field.Field orientation="horizontal">
              <Field.Content>
                <Field.Label
                  >統計を添える<Help
                    >自分のヒーローでそのアイテムが買われる割合（最近 30
                    日の試合）と、買われる時刻の平均を出します。めったに買われないアイテムには「珍しい」、半分以上の試合で買われるアイテムには「定番」と付けます</Help
                  ></Field.Label
                >
              </Field.Content>
              <Switch
                aria-label="統計を添える"
                checked={c.showRate}
                onCheckedChange={(showRate) => panel.update({ purchaseCards: { showRate } })}
              />
            </Field.Field>
          </Field.Group>
        </Card.Content>
      </Card.Root>
    {/if}
  </div>
</OverlayPage>
