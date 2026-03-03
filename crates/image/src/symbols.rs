#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolKind {
    Func,
    Object,
    Other,
}

#[derive(Debug, Clone)]
pub struct Symbol {
    pub name: String,
    pub addr: u32,
    pub size: u32,
    pub kind: SymbolKind,
}

pub struct SymbolTable {
    symbols: Vec<Symbol>,
}

impl SymbolTable {
    pub fn new(mut symbols: Vec<Symbol>) -> Self {
        symbols.sort_by_key(|s| s.addr);
        Self { symbols }
    }

    /// Find the symbol containing `addr` (i.e. `sym.addr <= addr < sym.addr + sym.size`)
    pub fn lookup(&self, addr: u32) -> Option<&Symbol> {
        let idx = self
            .symbols
            .partition_point(|s| s.addr <= addr)
            .checked_sub(1)?;
        let sym = &self.symbols[idx];
        if addr < sym.addr.wrapping_add(sym.size) {
            Some(sym)
        } else {
            None
        }
    }

    /// Find a symbol whose start address is exactly `addr`
    pub fn lookup_exact(&self, addr: u32) -> Option<&Symbol> {
        let idx = self.symbols.partition_point(|s| s.addr < addr);
        self.symbols.get(idx).filter(|s| s.addr == addr)
    }

    /// Returns `true` if `addr` is the entry point of a function symbol
    pub fn is_function_entry(&self, addr: u32) -> bool {
        self.lookup_exact(addr)
            .is_some_and(|s| s.kind == SymbolKind::Func)
    }
}
