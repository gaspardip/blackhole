function update(Object) { const define = Object.defineProperty; define(navigator, "clipboard", { value: clipboard }); }
