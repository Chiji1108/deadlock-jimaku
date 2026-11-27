import { mount } from "svelte";
import "../../lib/lang.css";
import { fitSource, PURCHASES_SIZE } from "../../lib/overlay-size";
import Purchases from "./Purchases.svelte";

fitSource(PURCHASES_SIZE, (scale) =>
  document.documentElement.style.setProperty("--scale", String(scale)),
);

mount(Purchases, { target: document.getElementById("app")! });
