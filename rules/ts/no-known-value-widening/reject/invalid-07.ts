type Command = () => void; const startCommand = () => {}; const commands: { [K in string]: Command } = { start: startCommand };
