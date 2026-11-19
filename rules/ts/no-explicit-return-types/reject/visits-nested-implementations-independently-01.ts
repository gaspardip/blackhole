function outer(): number { const inner = (): number => 1; return inner(); }
