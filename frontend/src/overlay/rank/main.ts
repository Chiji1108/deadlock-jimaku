import { mount } from "svelte";
import "../../lib/lang.css";
import { fitWidth } from "../../lib/fit-width";
import Rank from "./Rank.svelte";

// ブラウザソースの既定の幅（操作パネルの RANK_SIZE）
fitWidth(480);

mount(Rank, { target: document.getElementById("app")! });
