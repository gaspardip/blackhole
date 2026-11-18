const { navigator: host } = window; const { clipboard: board } = host; Reflect.set(board, "readText", read);
