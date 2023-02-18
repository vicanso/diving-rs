import type { Component, Accessor, Signal } from 'solid-js';
import { createSignal, createRenderEffect } from "solid-js";
import axios from 'axios';

import logo from './logo.png';
import styles from './App.module.css';

function model(element: HTMLInputElement, value: Accessor<Signal<string>>) {
  const [field, setField] = value();
  createRenderEffect(() => (element.value = field()));

  element.addEventListener("input", (e) => setField(e.target.value));
}
declare module "solid-js" {
  namespace JSX {
    interface Directives {  // use:model
      model: Signal<string>;
    }
  }
}

const App: Component = () => {
  const [name, setName] = createSignal("");
  const analyze = async () => {
    let image = name();
    try {
      const resp = await axios.get(`/api/analyze?image=${image}`);
      console.dir(resp);
    } catch (err) {
      console.error(err);
    }
  };
  const onkeydown = (e: KeyboardEvent) => {
    if (e.code === "Enter") {
      analyze();
    }
  };

  return (
    <div class={styles.App}>
      <header class={styles.header}>
        <div class={styles.logo}>

          <img src={logo} alt="logo" />
          <a
            href="https://github.com/solidjs/solid"
            target="_blank"
            rel="noopener noreferrer"
          >
            Diving
          </a>
        </div>
        <div class={styles.search}>
          <a href="#" onClick={analyze}><svg viewBox="64 64 896 896"><path d="M909.6 854.5 649.9 594.8C690.2 542.7 712 479 712 412c0-80.2-31.3-155.4-87.9-212.1-56.6-56.7-132-87.9-212.1-87.9s-155.5 31.3-212.1 87.9C143.2 256.5 112 331.8 112 412c0 80.1 31.3 155.5 87.9 212.1C256.5 680.8 331.8 712 412 712c67 0 130.6-21.8 182.7-62l259.7 259.6a8.2 8.2 0 0 0 11.6 0l43.6-43.5a8.2 8.2 0 0 0 0-11.6zM570.4 570.4C528 612.7 471.8 636 412 636s-116-23.3-158.4-65.6C211.3 528 188 471.8 188 412s23.3-116.1 65.6-158.4C296 211.3 352.2 188 412 188s116.1 23.2 158.4 65.6S636 352.2 636 412s-23.3 116.1-65.6 158.4z"></path></svg></a>
          <input placeholder='Input the name of image' type='text' use:model={[name, setName]} onkeydown={onkeydown} />
        </div>
      </header>
    </div>
  );
};

export default App;
