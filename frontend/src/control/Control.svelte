<script lang="ts">
  import { onMount } from "svelte";
  import type { KeySource } from "../generated/KeySource";
  import type { LanguageOption } from "../generated/LanguageOption";
  import type { Provider } from "../generated/Provider";
  import type { SettingsPatch } from "../generated/SettingsPatch";
  import type { SettingsView } from "../generated/SettingsView";
  import { api } from "../lib/api";
  import { connect, type Status } from "../lib/ws";

  const MAX_TRANSLATE_TO = 4;

  let status = $state<Status>("connecting");
  let view = $state<SettingsView | undefined>();
  let languages = $state<LanguageOption[]>([]);
  let devices = $state<string[]>([]);
  let error = $state<string | undefined>();
  /** 字幕を始められなかった・止まった理由 */
  let transcriptionError = $state<string | undefined>();
  let copied = $state<string | undefined>();
  let rankInput = $state("");
  let keyInputs = $state<Record<Provider, string>>({ mistral: "", openai: "" });

  const overlays = {
    subtitles: `${location.origin}/overlay/subtitles`,
    rank: `${location.origin}/overlay/rank`,
  };
  const label = (code: string) => languages.find((l) => l.code === code)?.label ?? code;

  onMount(() => {
    void load();
    return connect(
      ["status"],
      (msg) => {
        if (msg.type === "transcription.status") transcriptionError = msg.error ?? undefined;
      },
      (s) => (status = s),
    );
  });

  async function load() {
    await run(async () => {
      [view, languages, devices] = await Promise.all([
        api.settings(),
        api.languages(),
        api.devices().catch(() => []),
      ]);
      rankInput = view.settings.rankAccount?.toString() ?? "";
    });
  }

  /** 失敗したらサーバーの説明を出す */
  async function run(action: () => Promise<void>) {
    try {
      await action();
      error = undefined;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  const update = (patch: SettingsPatch) =>
    run(async () => {
      view = await api.updateSettings(patch);
    });

  function toggleTarget(code: string, on: boolean) {
    if (!view) return;
    const current = view.settings.translateTo;
    void update({ translateTo: on ? [...current, code] : current.filter((c) => c !== code) });
  }

  const saveKey = (provider: Provider, key: string) =>
    run(async () => {
      view = await api.setKey(provider, key);
      keyInputs[provider] = "";
    });

  async function copy(url: string) {
    await navigator.clipboard.writeText(url);
    copied = url;
    setTimeout(() => (copied = undefined), 1500);
  }

  const sourceLabel = (source: KeySource | undefined) =>
    source === "env" ? "環境変数" : source === "keyring" ? "保存済み" : "未設定";
  const providers: { provider: Provider; name: string }[] = [
    { provider: "mistral", name: "Mistral（文字起こし）" },
    { provider: "openai", name: "OpenAI（翻訳・読み）" },
  ];
</script>

<main>
  <h1>Deadlock Stream Overlays</h1>
  <p class="status {status}">
    サーバー: {status === "open" ? "接続中" : status === "connecting" ? "接続しています…" : "切断"}
  </p>
  {#if error}<p class="error">{error}</p>{/if}
  {#if view?.unsaved}<p class="error">{view.unsaved}</p>{/if}

  {#snippet url(href: string)}
    <div class="url">
      <code>{href}</code>
      <button onclick={() => copy(href)}>{copied === href ? "コピーしました" : "コピー"}</button>
    </div>
  {/snippet}

  {#if view}
    {@const s = view.settings}
    <section>
      <h2>字幕</h2>
      <div class="row">
        <button
          class="primary"
          class:on={s.transcriptionEnabled}
          disabled={!s.transcriptionEnabled && !view.keys.mistral}
          onclick={() => update({ transcriptionEnabled: !s.transcriptionEnabled })}
        >
          {s.transcriptionEnabled ? "■ 字幕を止める" : "● 字幕を始める"}
        </button>
        {#if !view.keys.mistral}<span class="hint">Mistral の API キーを設定してください</span>{/if}
      </div>
      {#if transcriptionError}
        <p class="error">字幕が止まっています: {transcriptionError}</p>
      {/if}

      <label>
        マイク
        <select
          value={s.micDevice ?? ""}
          onchange={(e) => update({ micDevice: e.currentTarget.value })}
        >
          <option value="">既定のデバイス</option>
          {#each devices as name (name)}<option value={name}>{name}</option>{/each}
        </select>
      </label>

      <label>
        話す言語
        <select
          value={s.micLanguage}
          onchange={(e) => update({ micLanguage: e.currentTarget.value })}
        >
          {#each languages as l (l.code)}<option value={l.code}>{l.label}</option>{/each}
        </select>
      </label>

      <fieldset>
        <legend>翻訳先（最大 {MAX_TRANSLATE_TO} つ。1 つ増えるごとに 1 時間あたり約 $0.3）</legend>
        <div class="langs">
          {#each languages.filter((l) => l.code !== s.micLanguage) as l (l.code)}
            {@const checked = s.translateTo.includes(l.code)}
            <label class="lang" class:checked>
              <input
                type="checkbox"
                {checked}
                disabled={!checked && s.translateTo.length >= MAX_TRANSLATE_TO}
                onchange={(e) => toggleTarget(l.code, e.currentTarget.checked)}
              />
              {l.label}
            </label>
          {/each}
        </div>
        {#if s.translateTo.length > 0 && !view.keys.openai}
          <p class="hint">翻訳するには OpenAI の API キーを設定してください</p>
        {/if}
      </fieldset>

      <h3>オーバーレイ</h3>
      <p>
        OBS のブラウザソースに設定してください。表示する訳文は <code>?langs=en,ru</code> で絞れます。
      </p>
      {@render url(overlays.subtitles)}
      <button onclick={() => api.demo()} disabled={status !== "open"}>テスト字幕を流す</button>
    </section>

    <section>
      <h2>ランク</h2>
      <form
        class="row"
        onsubmit={(e) => {
          e.preventDefault();
          void update({ rankAccount: rankInput });
        }}
      >
        <label class="grow">
          Steam ID（SteamID64 か 32bit のアカウント ID。空にすると表示しない）
          <input bind:value={rankInput} placeholder="76561198…" inputmode="numeric" />
        </label>
        <button type="submit">保存</button>
      </form>
      <p>
        ランク名の言語は <code>?lang=en</code> で変えられます（{[
          s.micLanguage,
          "en",
          ...s.translateTo,
        ]
          .map(label)
          .join("・")}）。
      </p>
      {@render url(overlays.rank)}
    </section>

    <section>
      <h2>API キー</h2>
      <p>OS の資格情報ストア（Windows の資格情報マネージャー、Mac のキーチェーン）に保存します。</p>
      {#each providers as k (k.provider)}
        {@const source = view.keys[k.provider]}
        <form
          class="row"
          onsubmit={(e) => {
            e.preventDefault();
            void saveKey(k.provider, keyInputs[k.provider]);
          }}
        >
          <label class="grow">
            <span
              >{k.name} <span class="badge {source ?? 'unset'}">{sourceLabel(source)}</span></span
            >
            <input
              type="password"
              autocomplete="off"
              bind:value={keyInputs[k.provider]}
              disabled={source === "env"}
              placeholder={source === "env" ? "環境変数が優先されます" : "新しいキーを貼り付け"}
            />
          </label>
          <button type="submit" disabled={!keyInputs[k.provider]}>保存</button>
          {#if source === "keyring"}
            <button type="button" onclick={() => saveKey(k.provider, "")}>削除</button>
          {/if}
        </form>
      {/each}
    </section>
  {/if}
</main>

<style>
  :global(body) {
    margin: 0;
    font-family: system-ui, sans-serif;
    background: #16181d;
    color: #e8e8ea;
  }
  main {
    max-width: 760px;
    margin: 0 auto;
    padding: 24px 16px 48px;
  }
  h1 {
    font-size: 20px;
  }
  h2 {
    font-size: 16px;
    margin: 0 0 12px;
  }
  h3 {
    font-size: 14px;
    margin: 20px 0 4px;
  }
  section {
    margin-top: 20px;
    padding: 16px;
    border: 1px solid #2b2f38;
    border-radius: 10px;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin: 12px 0;
    font-size: 13px;
    color: #b8bcc6;
  }
  fieldset {
    margin: 12px 0;
    border: none;
    padding: 0;
  }
  legend {
    font-size: 13px;
    color: #b8bcc6;
    margin-bottom: 6px;
  }
  .langs {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  label.lang {
    flex-direction: row;
    align-items: center;
    margin: 0;
    padding: 4px 8px;
    border: 1px solid #2b2f38;
    border-radius: 999px;
    color: #e8e8ea;
  }
  label.lang.checked {
    border-color: #6f8cff;
    background: #1e2640;
  }
  .row {
    display: flex;
    gap: 8px;
    align-items: flex-end;
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  select,
  input:not([type="checkbox"]) {
    padding: 6px 8px;
    border: 1px solid #3a3f4b;
    border-radius: 6px;
    background: #22252c;
    color: inherit;
    font: inherit;
  }
  .status.open {
    color: #6fdc8c;
  }
  .status.closed {
    color: #ff7b72;
  }
  .error {
    padding: 8px 12px;
    border-radius: 6px;
    background: #3a1d1f;
    color: #ffb4ae;
  }
  .hint {
    font-size: 13px;
    color: #e3b341;
  }
  .badge {
    display: inline-block;
    margin-left: 6px;
    padding: 0 6px;
    border-radius: 4px;
    font-size: 12px;
    background: #2b2f38;
  }
  .badge.keyring,
  .badge.env {
    background: #1f3a2a;
    color: #6fdc8c;
  }
  .url {
    display: flex;
    gap: 8px;
    align-items: center;
    margin-bottom: 12px;
  }
  code {
    padding: 2px 6px;
    background: #22252c;
    border-radius: 6px;
    overflow-wrap: anywhere;
  }
  .url code {
    flex: 1;
    min-width: 0;
    padding: 6px 8px;
  }
  button {
    padding: 6px 14px;
    border: 1px solid #3a3f4b;
    border-radius: 6px;
    background: #2b2f38;
    color: inherit;
    cursor: pointer;
  }
  button.primary {
    padding: 10px 18px;
    font-weight: 700;
    background: #1f6f3f;
    border-color: #2b8f55;
  }
  button.primary.on {
    background: #6f1f25;
    border-color: #8f2b33;
  }
  button:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
