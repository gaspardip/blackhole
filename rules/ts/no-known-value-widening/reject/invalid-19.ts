type Command = () => void; const startCommand = () => {}; type Open = Readonly<Record<string, Command>>; const source = { start: startCommand }; const commands: Open = source;
