type Command = () => void; const startCommand = () => {}; interface Commands { readonly start: Command } const commands: Commands = { start: startCommand };
