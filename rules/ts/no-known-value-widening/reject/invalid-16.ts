type Command = () => void; const startCommand = () => {}; type Open = Record<string, Command>; const source = { start: startCommand }; const commands: Open = source;
