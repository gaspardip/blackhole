type Command = () => void; const startCommand = () => {}; declare function make(): Record<string, Command>; const commands: Record<string, Command> = make();
