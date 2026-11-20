type Command = () => void; const startCommand = () => {}; function create(): { start: Command } { return { start: startCommand }; }
