type Command = () => void; const startCommand = () => {}; interface Commands { readonly start: Command } function create(): Commands { return { start: startCommand }; }
