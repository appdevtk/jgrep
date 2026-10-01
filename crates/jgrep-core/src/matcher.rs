use jaq_core::load::{Arena, File, Loader};
use jaq_core::{data, unwrap_valr, Ctx, Vars};
use jaq_json::Val;

pub struct Matcher {
    filter: jaq_core::Filter<data::JustLut<Val>>,
}

impl Matcher {
    pub fn compile(code: &str) -> Result<Self, String> {
        let defs = jaq_core::defs()
            .chain(jaq_std::defs())
            .chain(jaq_json::defs());
        let funs = jaq_core::funs()
            .chain(jaq_std::funs())
            .chain(jaq_json::funs());

        let loader = Loader::new(defs);
        let arena = Arena::default();
        let modules = loader
            .load(&arena, File { code, path: () })
            .map_err(|e| format!("{e:?}"))?;
        let filter = jaq_core::Compiler::default()
            .with_funs(funs)
            .compile(modules)
            .map_err(|e| format!("{e:?}"))?;
        Ok(Self { filter })
    }

    pub fn apply(&self, input: Val) -> Result<Vec<Val>, String> {
        let ctx = Ctx::<data::JustLut<Val>>::new(&self.filter.lut, Vars::new([]));
        self.filter
            .id
            .run((ctx, input))
            .map(unwrap_valr)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("{e}"))
    }
}

pub fn is_match(value: &Val) -> bool {
    !matches!(value, Val::Null | Val::Bool(false))
}
