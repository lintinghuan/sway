use crate::{
    language::{ty, CallPath, Visibility},
    Engines, Ident, TypeId,
};

use super::{module::Module, root::Root, submodule_namespace::SubmoduleNamespace, Path, PathBuf};

use sway_error::handler::{ErrorEmitted, Handler};
use sway_types::span::Span;

/// Enum used to pass a value asking for insertion of type into trait map when an implementation
/// of the trait cannot be found.
#[derive(Debug)]
pub enum TryInsertingTraitImplOnFailure {
    Yes,
    No,
}

/// The set of items that represent the namespace context passed throughout type checking.
#[derive(Clone, Debug)]
pub struct Namespace {
    /// An immutable namespace that consists of the names that should always be present, no matter
    /// what module or scope we are currently checking.
    ///
    /// These include external library dependencies and (when it's added) the `std` prelude.
    ///
    /// This is passed through type-checking in order to initialise the namespace of each submodule
    /// within the project.
    init: Module,
    /// The `root` of the project namespace.
    ///
    /// From the root, the entirety of the project's namespace can always be accessed.
    ///
    /// The root is initialised from the `init` namespace before type-checking begins.
    pub(crate) root: Root,
    /// An absolute path from the `root` that represents the current module being checked.
    ///
    /// E.g. when type-checking the root module, this is equal to `[]`. When type-checking a
    /// submodule of the root called "foo", this would be equal to `[foo]`.
    pub(crate) mod_path: PathBuf,
}

impl Namespace {
    /// Initialise the namespace at its root from the given initial namespace.
    pub fn init_root(init: Module) -> Self {
        let root = Root::from(init.clone());
        let mod_path = vec![];
        Self {
            init,
            root,
            mod_path,
        }
    }

    /// A reference to the path of the module currently being type-checked.
    pub fn mod_path(&self) -> &Path {
        &self.mod_path
    }

    /// Find the module that these prefixes point to
    pub fn find_module_path<'a>(
        &'a self,
        prefixes: impl IntoIterator<Item = &'a Ident>,
    ) -> PathBuf {
        self.mod_path.iter().chain(prefixes).cloned().collect()
    }

    /// A reference to the root of the project namespace.
    pub fn root(&self) -> &Root {
        &self.root
    }

    /// A mutable reference to the root of the project namespace.
    pub fn root_mut(&mut self) -> &mut Root {
        &mut self.root
    }

    /// Access to the current [Module], i.e. the module at the inner `mod_path`.
    ///
    /// Note that the [Namespace] will automatically dereference to this [Module] when attempting
    /// to call any [Module] methods.
    pub fn module(&self) -> &Module {
        &self.root.module[&self.mod_path]
    }

    /// Mutable access to the current [Module], i.e. the module at the inner `mod_path`.
    ///
    /// Note that the [Namespace] will automatically dereference to this [Module] when attempting
    /// to call any [Module] methods.
    pub fn module_mut(&mut self) -> &mut Module {
        &mut self.root.module[&self.mod_path]
    }

    /// Short-hand for calling [Root::resolve_symbol] on `root` with the `mod_path`.
    pub(crate) fn resolve_symbol(
        &self,
        handler: &Handler,
        engines: &Engines,
        symbol: &Ident,
        self_type: Option<TypeId>,
    ) -> Result<ty::TyDecl, ErrorEmitted> {
        self.root
            .resolve_symbol(handler, engines, &self.mod_path, symbol, self_type)
    }

    /// Short-hand for calling [Root::resolve_call_path] on `root` with the `mod_path`.
    pub(crate) fn resolve_call_path(
        &self,
        handler: &Handler,
        engines: &Engines,
        call_path: &CallPath,
        self_type: Option<TypeId>,
    ) -> Result<ty::TyDecl, ErrorEmitted> {
        self.root
            .resolve_call_path(handler, engines, &self.mod_path, call_path, self_type)
    }

    /// "Enter" the submodule at the given path by returning a new [SubmoduleNamespace].
    ///
    /// Here we temporarily change `mod_path` to the given `dep_mod_path` and wrap `self` in a
    /// [SubmoduleNamespace] type. When dropped, the [SubmoduleNamespace] resets the `mod_path`
    /// back to the original path so that we can continue type-checking the current module after
    /// finishing with the dependency.
    pub(crate) fn enter_submodule(
        &mut self,
        mod_name: Ident,
        visibility: Visibility,
        module_span: Span,
    ) -> SubmoduleNamespace {
        let init = self.init.clone();
        self.submodules.entry(mod_name.to_string()).or_insert(init);
        let submod_path: Vec<_> = self
            .mod_path
            .iter()
            .cloned()
            .chain(Some(mod_name.clone()))
            .collect();
        let parent_mod_path = std::mem::replace(&mut self.mod_path, submod_path);
        self.name = Some(mod_name);
        self.span = Some(module_span);
        self.visibility = visibility;
        self.is_external = false;
        SubmoduleNamespace {
            namespace: self,
            parent_mod_path,
        }
    }

    /// Returns true if the current module being checked is a direct or indirect submodule of
    /// the module given by the `absolute_module_path`.
    ///
    /// The current module being checked is determined by `mod_path`.
    ///
    /// E.g., the `mod_path` `[fist, second, third]` of the root `foo` is a submodule of the module
    /// `[foo, first]`. Note that the `mod_path` does not contain the root name, while the
    /// `absolute_module_path` always contains it.
    ///
    /// If the current module being checked is the same as the module given by the `absolute_module_path`,
    /// the `true_if_same` is returned.
    pub(crate) fn module_is_submodule_of(
        &self,
        absolute_module_path: &Path,
        true_if_same: bool,
    ) -> bool {
        // `mod_path` does not contain the root name, so we have to separately check
        // that the root name is equal to the module package name.
        let root_name = match &self.root.name {
            Some(name) => name,
            None => panic!("Root module must always have a name."),
        };

        let (package_name, modules) = absolute_module_path.split_first().expect("Absolute module path must have at least one element, because it always contains the package name.");

        if root_name != package_name {
            return false;
        }

        if self.mod_path.len() < modules.len() {
            return false;
        }

        let is_submodule = modules
            .iter()
            .zip(self.mod_path.iter())
            .all(|(left, right)| left == right);

        if is_submodule {
            if self.mod_path.len() == modules.len() {
                true_if_same
            } else {
                true
            }
        } else {
            false
        }
    }

    /// Returns true if the module given by the `absolute_module_path` is external
    /// to the current package. External modules are imported in the `Forc.toml` file.
    pub(crate) fn module_is_external(&self, absolute_module_path: &Path) -> bool {
        let root_name = match &self.root.name {
            Some(name) => name,
            None => panic!("Root module must always have a name."),
        };

        assert!(!absolute_module_path.is_empty(), "Absolute module path must have at least one element, because it always contains the package name.");

        root_name != &absolute_module_path[0]
    }
}

impl std::ops::Deref for Namespace {
    type Target = Module;
    fn deref(&self) -> &Self::Target {
        self.module()
    }
}

impl std::ops::DerefMut for Namespace {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.module_mut()
    }
}
