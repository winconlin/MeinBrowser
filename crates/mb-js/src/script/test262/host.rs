//! Der Host für Test262 mit den Funktionen aus INTERPRETING.md
//! („Host-Defined Functions“): `print` und `$262` mit `createRealm`,
//! `detachArrayBuffer`, `evalScript`, `gc` und `global`. Es fehlen
//! `IsHTMLDDA` und `AbstractModuleSource`, denn die Engine hat die Teile
//! dafür noch nicht, und `agent`: Jeder weitere Agent bräuchte einen
//! eigenen Faden mit eigenem Heap. Die Tests, die mehrere Agents starten,
//! scheitern deshalb noch erwartet.
//!
//! Lesarten, wo INTERPRETING.md nichts festlegt: Die Eigenschaften von
//! `$262` haben dieselben Attribute wie `print` und `$262` selbst
//! (beschreibbar, konfigurierbar, nicht aufzählbar). `evalScript` und
//! `print` wandeln ihr erstes Argument mit ToString. `length` ist die Zahl
//! der Argumente, die die Funktion liest. `detachArrayBuffer` wirft für
//! etwas anderes als einen ArrayBuffer, auch für einen SharedArrayBuffer,
//! einen TypeError, wo die abstrakte Operation nur zusichert, und gibt
//! sonst *undefined* zurück.
//!
//! Module lädt der Host nach INTERPRETING.md („Modules“): Ein Bezeichner
//! beginnt mit `./`, der Rest ist der Name einer Datei im Verzeichnis des
//! Tests. Mit `type: "json"` wird sie ein JSON-Modul, ohne Attribut ein
//! Modul aus Quelltext; jeder andere Typ, ein anderer Bezeichner oder eine
//! fehlende Datei beenden das Laden mit einem TypeError, ein früher Fehler
//! mit einem SyntaxError. Jede Datei gibt je Typ einmal ein Modul, auch die
//! des Tests selbst, gleich ob ein Modul oder `import()` fragt; Code aus
//! einem Script fragt mit dem Realm als *referrer*. Kann der Übersetzer eine
//! Hilfsdatei nicht, merkt sich der Host das, und der Läufer zählt den Test
//! als nicht unterstützt.

use std::any::Any;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::agent::{Abrupt, Agent, Completion, ErrorKind};
use crate::buffer;
use crate::builtin::{Builtin, BuiltinCall, define_builtin_function};
use crate::conversion::to_string;
use crate::descriptor::PropertyDescriptor;
use crate::heap::{Local, Scope};
use crate::host::Host;
use crate::module::{LoadPayload, ModuleId, ModuleRequest, Referrer};
use crate::object::ordinary_object_create;
use crate::operations::{define_property_or_throw, get};
use crate::realm::{Intrinsic, RealmId};
use crate::script::ScriptError;
use crate::value::{Kind, Value};

/// Der Host eines Agents im Läufer. Er merkt sich, was `print` ausgab,
/// und die Module, die er geladen hat.
#[derive(Default)]
pub(super) struct Test262Host {
    printed: RefCell<Vec<Vec<u16>>>,
    modules: RefCell<Modules>,
}

/// Die Module eines Tests.
#[derive(Default)]
struct Modules {
    /// Das Verzeichnis des Tests.
    directory: Option<PathBuf>,
    /// Die geladenen Module nach Dateiname und ob als JSON.
    loaded: BTreeMap<(Vec<u16>, bool), ModuleId>,
    /// Ob eine Hilfsdatei den Übersetzer an eine Grenze brachte.
    unsupported: bool,
}

impl Test262Host {
    /// Die Ausgaben von `print` in ihrer Reihenfolge.
    pub(super) fn printed(&self) -> Vec<Vec<u16>> {
        self.printed.borrow().clone()
    }

    /// Merkt sich die Datei `file` des Tests; Bezeichner beziehen sich auf
    /// ihr Verzeichnis, auch für `import()` aus einem Script.
    pub(super) fn set_test_file(&self, file: &Path) {
        self.modules.borrow_mut().directory = file.parent().map(Path::to_path_buf);
    }

    /// Merkt sich das Modul des Tests in der Datei `file`.
    pub(super) fn set_test_module(&self, file: &Path, module: ModuleId) {
        self.set_test_file(file);
        if let Some(name) = file.file_name() {
            let name = units(&name.to_string_lossy());
            self.modules
                .borrow_mut()
                .loaded
                .insert((name, false), module);
        }
    }

    /// Ob der Übersetzer eine Hilfsdatei nicht übersetzen konnte.
    pub(super) fn module_unsupported(&self) -> bool {
        self.modules.borrow().unsupported
    }

    /// Das Modul zur Anfrage: ein schon geladenes oder ein neues aus der
    /// Datei.
    fn module_for(
        &self,
        agent: &mut Agent,
        scope: &Scope,
        realm: RealmId,
        request: &ModuleRequest,
    ) -> Completion<ModuleId> {
        let json = match request.attribute("type") {
            None => false,
            Some(kind) if kind == units("json") => true,
            Some(_) => return Err(agent.throw_error(ErrorKind::TypeError)),
        };
        let name = match request.specifier.strip_prefix(&units("./")[..]) {
            Some(name) if !name.is_empty() && !name.contains(&u16::from(b'/')) => name.to_vec(),
            _ => return Err(agent.throw_error(ErrorKind::TypeError)),
        };
        let key = (name, json);
        let (known, directory) = {
            let modules = self.modules.borrow();
            (modules.loaded.get(&key).copied(), modules.directory.clone())
        };
        if let Some(module) = known {
            return Ok(module);
        }
        let text = String::from_utf16(&key.0)
            .ok()
            .zip(directory)
            .and_then(|(name, directory)| std::fs::read(directory.join(name)).ok())
            .and_then(|bytes| String::from_utf8(bytes).ok());
        let Some(text) = text else {
            return Err(agent.throw_error(ErrorKind::TypeError));
        };
        let source = units(&text);
        let module = if json {
            agent.parse_json_module(scope, realm, &source)?
        } else {
            match agent.parse_module(scope, realm, &source) {
                Ok(module) => module,
                Err(ScriptError::Syntax(_)) => {
                    return Err(agent.throw_error(ErrorKind::SyntaxError));
                }
                Err(ScriptError::Unsupported(_)) => {
                    self.modules.borrow_mut().unsupported = true;
                    return Err(agent.throw_error(ErrorKind::TypeError));
                }
                Err(ScriptError::Abrupt(abrupt)) => return Err(abrupt),
            }
        };
        self.modules.borrow_mut().loaded.insert(key, module);
        Ok(module)
    }
}

static PRINT: Builtin = Builtin {
    name: "print",
    length: 1,
    constructor: false,
    behaviour: print,
};

static FUNCTIONS: [Builtin; 4] = [
    Builtin {
        name: "createRealm",
        length: 0,
        constructor: false,
        behaviour: create_realm,
    },
    Builtin {
        name: "detachArrayBuffer",
        length: 1,
        constructor: false,
        behaviour: detach_array_buffer,
    },
    Builtin {
        name: "evalScript",
        length: 1,
        constructor: false,
        behaviour: eval_script,
    },
    Builtin {
        name: "gc",
        length: 0,
        constructor: false,
        behaviour: gc,
    },
];

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

impl Host for Test262Host {
    fn define_global_properties(
        &self,
        agent: &mut Agent,
        scope: &Scope,
        realm: RealmId,
    ) -> Completion<()> {
        let global = agent.global_object(scope, realm);
        let prototype = agent.intrinsic(scope, realm, Intrinsic::ObjectPrototype);
        let object = ordinary_object_create(agent, scope, Some(prototype))?;
        for builtin in &FUNCTIONS {
            define_builtin_function(agent, scope, realm, object, builtin)?;
        }
        define(agent, scope, object, "global", global)?;
        define_builtin_function(agent, scope, realm, global, &PRINT)?;
        define(agent, scope, global, "$262", object)
    }

    fn load_imported_module(
        &self,
        agent: &mut Agent,
        scope: &Scope,
        referrer: Referrer,
        request: &ModuleRequest,
        payload: LoadPayload,
    ) -> Completion<()> {
        let realm = match referrer {
            Referrer::Module(module) => agent.module_realm(module),
            Referrer::Realm(realm) => Some(realm),
        };
        let result = match realm {
            Some(realm) => self.module_for(agent, scope, realm, request),
            None => Err(agent.throw_error(ErrorKind::TypeError)),
        };
        agent.finish_loading_imported_module(scope, referrer, request, payload, result)
    }

    fn supports_import_attribute(&self, key: &[u16]) -> bool {
        key == units("type")
    }
}

/// Eine beschreibbare, konfigurierbare, nicht aufzählbare Eigenschaft.
fn define(
    agent: &mut Agent,
    scope: &Scope,
    object: Local<'_>,
    name: &str,
    value: Local<'_>,
) -> Completion<()> {
    let key = agent.key_from_units(scope, &units(name))?;
    let descriptor = PropertyDescriptor::data(value, true, false, true);
    define_property_or_throw(agent, scope, object, key, &descriptor)
}

/// Die Codeeinheiten von ToString(`value`).
fn text(agent: &mut Agent, scope: &Scope, value: Local<'_>) -> Completion<Vec<u16>> {
    let string = to_string(agent, scope, value)?;
    let heap = agent.heap();
    Ok(match heap.get(string).kind() {
        Kind::String(string) => heap.string_units(string),
        _ => Vec::new(),
    })
}

/// `print(value)`: hält ToString(`value`) für den Läufer fest.
fn print<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    call: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let value = call.argument(agent, scope, 0);
    let text = text(agent, scope, value)?;
    let host = agent.host().and_then(|host| -> Option<Rc<Test262Host>> {
        let host: Rc<dyn Any> = host;
        host.downcast::<Test262Host>().ok()
    });
    let Some(host) = host else {
        return Err(agent.throw_error(ErrorKind::TypeError));
    };
    // ToString ist schon gelaufen; während der Ausleihe läuft nichts mehr
    // (RS-10).
    host.printed.borrow_mut().push(text);
    Ok(agent.heap().local(scope, Value::UNDEFINED))
}

/// `$262.createRealm()`: ein neuer Realm mit eigenem `$262`, das die
/// Funktion zurückgibt.
fn create_realm<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    _: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let realm = agent.create_realm()?;
    let global = agent.global_object(scope, realm);
    let key = agent.key_from_units(scope, &units("$262"))?;
    get(agent, scope, global, key)
}

/// `$262.evalScript(source)`: ParseScript und ScriptEvaluation im Realm der
/// Funktion, dem aktuellen Realm. Ein früher Fehler wird als SyntaxError
/// dieses Realms geworfen. Kann der Übersetzer etwas nicht, endet der ganze
/// Lauf mit [`Abrupt::Unsupported`]; die Lage bezieht sich dann auf den
/// Quelltext von `evalScript`.
fn eval_script<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    call: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let value = call.argument(agent, scope, 0);
    let source = text(agent, scope, value)?;
    match agent.evaluate_script(scope, call.realm, &source) {
        Ok(result) => Ok(result),
        Err(ScriptError::Syntax(_)) => Err(agent.throw_error(ErrorKind::SyntaxError)),
        Err(ScriptError::Unsupported(position)) => Err(Abrupt::Unsupported(position)),
        Err(ScriptError::Abrupt(abrupt)) => Err(abrupt),
    }
}

/// `$262.detachArrayBuffer(buffer)`: DetachArrayBuffer (Abschnitt
/// 25.1.3.5) ohne Schlüssel.
fn detach_array_buffer<'s>(
    agent: &mut Agent,
    scope: &'s Scope,
    arguments: &BuiltinCall<'_, '_>,
) -> Completion<Local<'s>> {
    let value = arguments.argument(agent, scope, 0);
    if !buffer::is_array_buffer(agent, value) {
        return Err(agent.throw_error(ErrorKind::TypeError));
    }
    buffer::detach_array_buffer(agent, value);
    Ok(agent.heap().local(scope, Value::UNDEFINED))
}

/// `$262.gc()`: eine Sammlung des Heaps.
fn gc<'s>(agent: &mut Agent, scope: &'s Scope, _: &BuiltinCall<'_, '_>) -> Completion<Local<'s>> {
    agent.heap_mut().collect();
    Ok(agent.heap().local(scope, Value::UNDEFINED))
}
