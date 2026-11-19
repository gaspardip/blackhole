type Command = () => void; const startCommand = () => {}; type Commands = { readonly start: Command }; const commands: Commands = { start: startCommand };
