use crate::symbols::{Symbol, SymbolKind, SymbolTable};
use object::read::elf::ElfFile32;
use object::{Endianness, Object, ObjectSymbol};

#[derive(Debug)]
pub enum ElfError {
    Parse(String),
}

impl std::fmt::Display for ElfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ElfError::Parse(msg) => write!(f, "ELF parse error: {msg}"),
        }
    }
}

impl std::error::Error for ElfError {}

pub fn parse_elf_symbols(data: &[u8]) -> Result<SymbolTable, ElfError> {
    let elf = ElfFile32::<Endianness>::parse(data).map_err(|e| ElfError::Parse(e.to_string()))?;

    let mut symbols = Vec::new();

    for sym in elf.symbols() {
        let name = sym.name().unwrap_or_default();
        if name.is_empty() || sym.address() == 0 {
            continue;
        }

        let kind = match sym.kind() {
            object::SymbolKind::Text => SymbolKind::Func,
            object::SymbolKind::Data => SymbolKind::Object,
            _ => SymbolKind::Other,
        };

        symbols.push(Symbol {
            name: name.to_string(),
            addr: sym.address() as u32,
            size: sym.size() as u32,
            kind,
        });
    }

    Ok(SymbolTable::new(symbols))
}
