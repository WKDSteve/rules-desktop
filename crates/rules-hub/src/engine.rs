//! The engine seam of the rules hub: the seam itself is `appkit_hub::engine`; what is the rules
//! engine's own is here: its calls (`rules-export/1`), its dialect and how a setup becomes a
//! `CliEngine`.
//!
//! A [`Method`] is a closed list, never a string from outside: the only way from a name to a call
//! is [`Calls::parse`]. The one call that writes is `ops`, the door of `rules-ops/1`; which of its
//! operations the page may ask for is the hub's list (`write::OPS_ALLOWED`), not this table's.

pub use appkit_hub::engine::*;

use appkit_hub::discovery::{engine_script, DiscoverySpec, Setup};
use appkit_hub::env::Env;

/// The contract version this hub speaks.
pub const SCHEMA: u64 = 1;

/// What the read side of the app needs from the engine.
pub const REQUIRED_CAPABILITIES: &[&str] = &["hello", "snapshot", "rule"];

/// What the seam must know about the rules engine.
///
/// `extra_areas_var` stays unused: the seam joins extra areas with `,`, and rules.nvim splits
/// `$RULES_RULESETS` at `;` and `:`, so rule sets outside the vault cannot be passed this way.
/// [`cli_config`] always passes none; the hub reads the rule sets of the vault only.
pub const DIALECT: EngineDialect = EngineDialect {
    schema: SCHEMA,
    schema_tag: None,
    error_kind: "rules.error",
    host_script: "rules-host.lua",
    vault_var: "RULES_VAULT",
    extra_areas_var: "RULES_RULESETS",
    required_capabilities: REQUIRED_CAPABILITIES,
    hello: Calls::Hello,
    engine_name: "the rules.nvim checkout",
};

/// What the hub looks for to start the engine: its variables, the checkout and the script in it.
pub const DISCOVERY: DiscoverySpec = DiscoverySpec {
    vault_var: "RULES_VAULT",
    nvim_var: "RULES_NVIM",
    engine_dir_var: "RULES_ENGINE_DIR",
    engine_name: "rules.nvim",
    engine_script: &["scripts", "rules.lua"],
};

/// Ask `hello` and check the answer against what this app needs.
///
/// # Errors
///
/// A [`ProbeProblem`] when the engine does not answer, speaks another contract version or lacks
/// a capability.
pub async fn probe(engine: &dyn Engine) -> Result<EngineInfo, ProbeProblem> {
    appkit_hub::engine::probe(engine, &DIALECT).await
}

/// The configuration of a `CliEngine` for what the setup found. No extra rule sets, whatever the
/// setup holds (see [`DIALECT`]).
#[must_use]
pub fn cli_config(setup: &Setup, env: Env) -> CliEngineConfig {
    CliEngineConfig {
        dialect: DIALECT,
        nvim: setup.nvim.value.clone(),
        script: engine_script(&DISCOVERY, &setup.engine_dir.value),
        vault: setup.vault.value.clone(),
        lib_dir: setup.lib_dir.as_ref().map(|l| l.value.clone()),
        extra_areas: Vec::new(),
        env,
        timeout: std::time::Duration::from_secs(30),
        max_queue: 64,
        // A document of the engine is at most 8 MiB (rules.nvim docs/CONTRACT.md).
        max_reply_bytes: 16 * 1024 * 1024,
    }
}

/// The calls of `rules-export/1` and `rules-ops/1` this hub makes. The seam reads the flags of each
/// entry; it knows no call by name. `bodies` is not a call of the engine: it names the pages of rule
/// text the search index asks for, answered inside the hub from `snapshot` and `rule`, and never
/// sent. `ops` is the one that writes; its answer is a `rules.result`.
static SPECS: [MethodSpec; 8] = [
    MethodSpec::bare("hello", "rules.hello"),
    MethodSpec::bare("snapshot", "rules.snapshot"),
    MethodSpec::read("rule", "rules.rule"),
    MethodSpec::read("bodies", "rules.bodies"),
    MethodSpec::read("next_id", "rules.next_id"),
    MethodSpec::write("ops", "rules.result"),
    MethodSpec::read("waivers", "rules.waivers"),
    MethodSpec::bare("rulesets", "rules.rulesets"),
];

/// The calls of the rules engine, by name.
pub struct Calls;

#[allow(non_upper_case_globals)]
impl Calls {
    pub const Hello: Method = Method::new(&SPECS[0]);
    pub const Snapshot: Method = Method::new(&SPECS[1]);
    pub const Rule: Method = Method::new(&SPECS[2]);
    pub const Bodies: Method = Method::new(&SPECS[3]);
    pub const NextId: Method = Method::new(&SPECS[4]);
    pub const Ops: Method = Method::new(&SPECS[5]);
    pub const Waivers: Method = Method::new(&SPECS[6]);
    pub const Rulesets: Method = Method::new(&SPECS[7]);

    /// Every call of the table.
    pub const ALL: [Method; 8] = [
        Self::Hello,
        Self::Snapshot,
        Self::Rule,
        Self::Bodies,
        Self::NextId,
        Self::Ops,
        Self::Waivers,
        Self::Rulesets,
    ];

    /// Parse the name of a route or a request. Nothing else is accepted.
    #[must_use]
    pub fn parse(name: &str) -> Option<Method> {
        Method::find(&Self::ALL, name)
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use appkit_hub::config::{Resolved, Source};

    use super::*;

    #[test]
    fn only_ops_writes_and_needs_a_request_and_none_runs_aside() {
        for m in Calls::ALL {
            assert_eq!(m.writes(), m == Calls::Ops, "{}", m.as_str());
            assert_eq!(m.needs_params(), m == Calls::Ops, "{}", m.as_str());
            assert!(!m.is_aside(), "{}", m.as_str());
        }
    }

    #[test]
    fn hello_snapshot_and_rulesets_take_no_parameters() {
        let bare: Vec<_> = Calls::ALL
            .iter()
            .filter(|m| !m.takes_params())
            .map(|m| m.as_str())
            .collect();
        assert_eq!(bare, ["hello", "snapshot", "rulesets"]);
    }

    #[test]
    fn every_name_round_trips_and_every_answer_kind_is_the_rules_engines() {
        for m in Calls::ALL {
            assert_eq!(Calls::parse(m.as_str()), Some(m));
            // The write door answers with a `rules.result`, the other calls with `rules.<name>`.
            let want = if m == Calls::Ops {
                "rules.result".to_owned()
            } else {
                format!("rules.{}", m.as_str())
            };
            assert_eq!(m.reply_kind(), want);
        }
        for bad in ["", "Rule", "rules.rule", "Ops", "check", "list", "Waivers"] {
            assert_eq!(Calls::parse(bad), None, "{bad:?}");
        }
    }

    /// These names are in the contract of rules.nvim and in the environments of people.
    #[test]
    fn the_dialect_names_the_rules_engine() {
        assert_eq!(DIALECT.schema, 1);
        assert_eq!(DIALECT.error_kind, "rules.error");
        assert_eq!(DIALECT.host_script, "rules-host.lua");
        assert_eq!(DIALECT.vault_var, "RULES_VAULT");
        assert_eq!(DIALECT.hello, Calls::Hello);
        for needed in REQUIRED_CAPABILITIES {
            assert!(Calls::parse(needed).is_some(), "{needed}");
        }
        assert_eq!(DISCOVERY.vault_var, DIALECT.vault_var);
    }

    fn resolved(path: &str) -> Resolved<PathBuf> {
        Resolved {
            value: PathBuf::from(path),
            source: Source::Setting,
        }
    }

    #[test]
    fn the_engine_is_started_with_the_vault_and_no_extra_rule_sets() {
        let setup = Setup {
            nvim: resolved("/opt/nvim/bin/nvim"),
            nvim_version: (0, 12, 2),
            engine_dir: resolved("/src/rules.nvim"),
            lib_dir: Some(resolved("/src/lib.nvim")),
            lib_problem: None,
            vault: resolved("/vault"),
            extra_areas: vec!["/elsewhere/a".to_owned(), "/elsewhere/b".to_owned()],
        };
        let cfg = cli_config(&setup, Env::from_pairs::<_, &str, &str>([]));
        assert_eq!(
            cfg.script,
            Path::new("/src/rules.nvim")
                .join("scripts")
                .join("rules.lua")
        );
        assert_eq!(cfg.vault, Path::new("/vault"));
        assert_eq!(cfg.lib_dir.as_deref(), Some(Path::new("/src/lib.nvim")));
        assert!(cfg.extra_areas.is_empty(), "{:?}", cfg.extra_areas);
        assert_eq!(cfg.dialect.error_kind, "rules.error");
    }
}
