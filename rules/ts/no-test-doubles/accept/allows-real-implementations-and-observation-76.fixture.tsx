function instrument(Object) { Object.defineProperty.call(Object, navigator, "clipboard", { value: clipboard }); }
