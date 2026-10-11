//! Security: cryptography, injection, transport security, credentials.

use crate::framework::*;
use crate::util::*;
use crate::{declare_rule, rule_meta};
use grounds_index::*;
use grounds_semantic::MethodMatcher;
use grounds_syntax::{Node, NodeExt};

pub fn rules() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(HardcodedCredentials),
        Box::new(WeakHash),
        Box::new(WeakCipher),
        Box::new(InsecureCipherMode),
        Box::new(WeakTls),
        Box::new(TrustAllCertificates),
        Box::new(HostnameVerifierAcceptsAll),
        Box::new(PredictableSecureRandom),
        Box::new(SqlInjection),
        Box::new(CommandInjection),
        Box::new(XxeVulnerable),
        Box::new(CsrfDisabled),
        Box::new(PermissiveCors),
        Box::new(WeakRsaKey),
        Box::new(InsecureCookie),
        Box::new(ObjectDeserialization),
        Box::new(StaticIv),
        Box::new(WorldWritablePermissions),
        Box::new(InsecureTempFile),
        Box::new(CleartextUrl),
    ]
}

fn credential_name(name: &str) -> bool {
    let l = name.to_ascii_lowercase().replace('_', "");
    ["password", "passwd", "pwd", "secret", "apikey", "accesskey", "privatekey", "token", "credential", "credentials", "authkey", "clientsecret"].iter().any(|k| l.ends_with(k))
}

fn looks_like_secret_value(v: &str) -> bool {
    if v.len() < 4 || v.contains(' ') {
        return false;
    }
    // placeholders and property references
    if v.starts_with("${") || v.starts_with('#') || v.starts_with('{') || v.starts_with('<') || v.contains("%s") || v.contains("{}") {
        return false;
    }
    let l = v.to_ascii_lowercase();
    if ["password", "secret", "changeme", "xxx", "***", "token", "redacted", "n/a", "none", "null", "dummy"].iter().any(|p| l == *p) {
        return false;
    }
    // looks like a key name rather than a value: "db.password", "X-Api-Token",
    // "AccessKey.Create", ".certPrivateKeyPassword", "provisionDeviceSecret"
    let word_chars = v.chars().all(|c| c.is_ascii_alphabetic() || c == '.' || c == '-' || c == '_');
    if word_chars && (v.contains('.') || v.contains('-')) {
        return false;
    }
    let humps = v.chars().zip(v.chars().skip(1)).filter(|(a, b)| a.is_ascii_lowercase() && b.is_ascii_uppercase()).count();
    if word_chars && humps >= 2 {
        return false;
    }
    // HTTP header names used as constants
    if ["authorization", "proxy-authorization", "cookie", "set-cookie", "www-authenticate"].contains(&l.as_str()) {
        return false;
    }
    true
}

declare_rule!(HardcodedCredentials {
    id: "hardcoded-credentials",
    name: "Hard-coded credential",
    category: Security,
    severity: Warning,
    scope: Main,
    sources: ["sonar:S2068", "sonar:S6418", "spotbugs:DMI_CONSTANT_DB_PASSWORD", "spotbugs:HARD_CODE_PASSWORD"],
    kinds: ["variable_declarator", "assignment_expression"],
    description: "Passwords, tokens and keys in source code end up in version control, binaries and logs, and cannot be rotated without a release. Load them from configuration or a secret store.",
});

impl Rule for HardcodedCredentials {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let (name, value) = if n.kind() == "variable_declarator" {
            (n.field("name").map(|x| cx.text(x)), n.field("value"))
        } else {
            (n.field("left").map(|l| cx.text(l).rsplit('.').next().unwrap_or("")), n.field("right"))
        };
        let (Some(name), Some(value)) = (name, value) else { return };
        if !credential_name(name) || value.unparen().kind() != "string_literal" {
            return;
        }
        let Some(v) = cx.fm.string_value(value) else { return };
        // the value is the name of the setting itself ("allow-credentials", "password")
        let norm = |s: &str| s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
        let (nv, nn) = (norm(&v), norm(name));
        if nn.contains(&nv) || nv.contains(&nn) {
            return;
        }
        if looks_like_secret_value(&v) {
            cx.report(value, format!("Hard-coded credential in `{name}`; load it from configuration"));
        }
    }
}

const MESSAGE_DIGEST: MethodMatcher = MethodMatcher::statics("java.security.MessageDigest").named(&["getInstance"]);
const DIGEST_UTILS_WEAK: MethodMatcher = MethodMatcher::statics("org.apache.commons.codec.digest.DigestUtils").named(&["md5", "md5Hex", "sha", "sha1", "sha1Hex", "shaHex", "md2", "md2Hex", "getMd5Digest", "getSha1Digest"]);

declare_rule!(WeakHash {
    id: "weak-hash-algorithm",
    name: "Weak hashing algorithm",
    category: Security,
    severity: Info,
    sources: ["sonar:S4790", "sonar:S2070", "spotbugs:WEAK_MESSAGE_DIGEST_MD5", "spotbugs:WEAK_MESSAGE_DIGEST_SHA1"],
    kinds: ["method_invocation"],
    description: "MD2, MD4, MD5 and SHA-1 have practical collision attacks. Use SHA-256 or better (and a dedicated algorithm like bcrypt/scrypt/Argon2/PBKDF2 for passwords).",
});

impl Rule for WeakHash {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if cx.fm.call_matches(n, &DIGEST_UTILS_WEAK) {
            cx.report(n, "Weak hash function (MD5/SHA-1)");
            return;
        }
        if !cx.fm.call_matches(n, &MESSAGE_DIGEST) {
            return;
        }
        let Some(a) = cx.fm.args_of(n).first().copied() else { return };
        let Some(alg) = cx.fm.string_value(a) else { return };
        let u = alg.to_ascii_uppercase().replace('-', "");
        if matches!(u.as_str(), "MD2" | "MD4" | "MD5" | "SHA" | "SHA1") {
            cx.report(a, format!("`{alg}` is a weak hash algorithm; use SHA-256 or stronger"));
        }
    }
}

const CIPHER_GET: MethodMatcher = MethodMatcher::statics("javax.crypto.Cipher").named(&["getInstance"]);
const KEYGEN_GET: MethodMatcher = MethodMatcher::statics("javax.crypto.KeyGenerator").named(&["getInstance"]);
const SECRET_KEY_FACTORY: MethodMatcher = MethodMatcher::statics("javax.crypto.SecretKeyFactory").named(&["getInstance"]);

declare_rule!(WeakCipher {
    id: "weak-cipher-algorithm",
    name: "Weak encryption algorithm",
    category: Security,
    severity: Error,
    sources: ["sonar:S5547", "spotbugs:DES_USAGE", "spotbugs:TDES_USAGE", "spotbugs:CIPHER_INTEGRITY"],
    kinds: ["method_invocation"],
    description: "DES, 3DES, RC2, RC4 and Blowfish are broken or have too-small keys/blocks. Use AES (GCM) or ChaCha20-Poly1305.",
});

impl Rule for WeakCipher {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !(cx.fm.call_matches(n, &CIPHER_GET) || cx.fm.call_matches(n, &KEYGEN_GET) || cx.fm.call_matches(n, &SECRET_KEY_FACTORY)) {
            return;
        }
        let Some(a) = cx.fm.args_of(n).first().copied() else { return };
        let Some(t) = cx.fm.string_value(a) else { return };
        let alg = t.split('/').next().unwrap_or("").to_ascii_uppercase();
        if matches!(alg.as_str(), "DES" | "DESEDE" | "TRIPLEDES" | "3DES" | "RC2" | "RC4" | "ARCFOUR" | "BLOWFISH") {
            cx.report(a, format!("`{t}` uses a weak cipher; use AES/GCM"));
        }
    }
}

declare_rule!(InsecureCipherMode {
    id: "insecure-cipher-mode",
    name: "Insecure cipher mode or padding",
    category: Security,
    severity: Error,
    sources: ["sonar:S5542", "spotbugs:ECB_MODE", "spotbugs:PADDING_ORACLE", "spotbugs:RSA_NO_PADDING"],
    kinds: ["method_invocation"],
    description: "ECB mode leaks patterns of the plaintext (and `Cipher.getInstance(\"AES\")` defaults to ECB); CBC with PKCS5 padding enables padding-oracle attacks; RSA without OAEP is malleable. Use `AES/GCM/NoPadding` and `RSA/ECB/OAEPWithSHA-256AndMGF1Padding`.",
});

impl Rule for InsecureCipherMode {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !cx.fm.call_matches(n, &CIPHER_GET) {
            return;
        }
        let Some(a) = cx.fm.args_of(n).first().copied() else { return };
        let Some(t) = cx.fm.string_value(a) else { return };
        let parts: Vec<String> = t.split('/').map(|p| p.to_ascii_uppercase()).collect();
        let alg = parts[0].as_str();
        let mode = parts.get(1).map(|s| s.as_str());
        let pad = parts.get(2).map(|s| s.as_str()).unwrap_or("");
        let problem = match (alg, mode) {
            ("AES", None) => Some("defaults to ECB mode"),
            ("RSA", _) if !pad.contains("OAEP") => Some("RSA without OAEP padding"),
            (_, Some("ECB")) if alg != "RSA" => Some("ECB mode"),
            (_, Some("CBC")) if pad.contains("PKCS5") || pad.contains("PKCS7") => Some("CBC with padding is vulnerable to padding oracles"),
            _ => None,
        };
        if let Some(p) = problem {
            let instead = if alg == "RSA" { "RSA/ECB/OAEPWithSHA-256AndMGF1Padding" } else { "AES/GCM/NoPadding" };
            cx.report(a, format!("`{t}`: {p}; use {instead}"));
        }
    }
}

const SSL_CONTEXT: MethodMatcher = MethodMatcher::statics("javax.net.ssl.SSLContext").named(&["getInstance"]);

declare_rule!(WeakTls {
    id: "weak-tls-protocol",
    name: "Weak SSL/TLS protocol",
    category: Security,
    severity: Error,
    sources: ["sonar:S4423", "spotbugs:SSL_CONTEXT"],
    kinds: ["method_invocation"],
    description: "SSLv2/3 and TLS 1.0/1.1 are deprecated and vulnerable (POODLE, BEAST). Use TLSv1.2 or TLSv1.3.",
});

impl Rule for WeakTls {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let is_ctx = cx.fm.call_matches(n, &SSL_CONTEXT);
        let is_protocols = matches!(cx.fm.call_name(n), Some("setEnabledProtocols" | "protocols"));
        if !is_ctx && !is_protocols {
            return;
        }
        for a in cx.fm.args_of(n) {
            let vals: Vec<String> = if a.kind() == "array_creation_expression" || a.kind() == "array_initializer" {
                a.descendants().filter(|d| d.kind() == "string_literal").filter_map(|d| grounds_syntax::string_literal_value(d, cx.src)).collect()
            } else {
                cx.fm.string_value(a).into_iter().collect()
            };
            for v in vals {
                // plain "SSL" is a context alias that negotiates TLS 1.2/1.3, like "TLS"
                if matches!(v.as_str(), "SSLv2" | "SSLv3" | "TLSv1" | "TLSv1.1" | "SSLv2Hello") {
                    cx.report(a, format!("`{v}` is an insecure protocol; use TLSv1.2+"));
                    return;
                }
            }
        }
    }
}

declare_rule!(TrustAllCertificates {
    id: "trust-all-certificates",
    name: "TrustManager that accepts every certificate",
    category: Security,
    severity: Error,
    sources: ["sonar:S4830", "spotbugs:WEAK_TRUST_MANAGER"],
    kinds: ["method_declaration"],
    description: "A `checkServerTrusted`/`checkClientTrusted` that does nothing disables certificate validation, allowing anyone to intercept the TLS connection (man-in-the-middle).",
});

impl Rule for TrustAllCertificates {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let name = method_name(n, cx.src);
        if !matches!(name, "checkServerTrusted" | "checkClientTrusted") {
            return;
        }
        let Some(cls) = cx.fm.enclosing_class(n) else { return };
        if !cx.fm.is_subclass(&cls.name, "javax.net.ssl.X509TrustManager").is_yes() && !cx.fm.is_subclass(&cls.name, "javax.net.ssl.TrustManager").is_yes() {
            return;
        }
        if n.field("body").is_some_and(|b| b.kids().is_empty()) {
            if let Some(nm) = n.field("name") {
                cx.report(nm, format!("`{name}` accepts every certificate"));
            }
        }
    }
}

declare_rule!(HostnameVerifierAcceptsAll {
    id: "hostname-verifier-accepts-all",
    name: "HostnameVerifier that accepts every host",
    category: Security,
    severity: Error,
    sources: ["sonar:S5527", "spotbugs:WEAK_HOSTNAME_VERIFIER"],
    kinds: ["method_declaration", "lambda_expression"],
    description: "A hostname verifier returning `true` lets any certificate impersonate any server.",
});

impl Rule for HostnameVerifierAcceptsAll {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if n.kind() == "lambda_expression" {
            let t = cx.fm.expected_type(n);
            if t.is_class("javax.net.ssl.HostnameVerifier") && n.field("body").is_some_and(|b| cx.text(b).trim() == "true" || cx.text(b).replace(char::is_whitespace, "") == "{returntrue;}") {
                cx.report(n, "HostnameVerifier accepts every host");
            }
            return;
        }
        if method_name(n, cx.src) != "verify" || param_count(n) != 2 {
            return;
        }
        let Some(cls) = cx.fm.enclosing_class(n) else { return };
        if !cx.fm.is_subclass(&cls.name, "javax.net.ssl.HostnameVerifier").is_yes() {
            return;
        }
        // the body must be exactly `return true;` (not any expression mentioning true)
        if n.field("body").is_some_and(|b| cx.text(b).replace(char::is_whitespace, "") == "{returntrue;}") {
            if let Some(nm) = n.field("name") {
                cx.report(nm, "HostnameVerifier accepts every host");
            }
        }
    }
}

declare_rule!(PredictableSecureRandom {
    id: "predictable-secure-random",
    name: "SecureRandom with a constant seed",
    category: Security,
    severity: Error,
    sources: ["sonar:S4347", "spotbugs:PREDICTABLE_RANDOM"],
    kinds: ["object_creation_expression", "method_invocation"],
    description: "Seeding SecureRandom with a constant makes its output predictable, defeating its purpose. Let it seed itself.",
});

impl Rule for PredictableSecureRandom {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let args = cx.fm.args_of(n);
        let Some(a) = args.first() else { return };
        let constant = cx.fm.constant_value(*a).is_some() || a.kind() == "array_creation_expression" || (a.kind() == "method_invocation" && cx.fm.call_name(*a) == Some("getBytes") && a.field("object").is_some_and(|o| cx.fm.constant_value(o).is_some()));
        if !constant {
            return;
        }
        let is_sr = if n.kind() == "object_creation_expression" {
            cx.type_of(n).is_class("java.security.SecureRandom")
        } else {
            cx.fm.call_name(n) == Some("setSeed") && n.field("object").is_some_and(|o| cx.type_of(o).is_class("java.security.SecureRandom"))
        };
        if is_sr {
            cx.report(n, "SecureRandom seeded with a constant is predictable");
        }
    }
}

const SQL_SINKS: &[MethodMatcher] = &[
    MethodMatcher::instance("java.sql.Statement").named(&["execute", "executeQuery", "executeUpdate", "executeLargeUpdate", "addBatch"]),
    MethodMatcher::instance("java.sql.Connection").named(&["prepareStatement", "prepareCall", "nativeSQL"]),
    MethodMatcher::instance("org.springframework.jdbc.core.JdbcTemplate").named(&["execute", "query", "queryForList", "queryForObject", "queryForMap", "queryForRowSet", "update", "batchUpdate"]),
    MethodMatcher::instance("jakarta.persistence.EntityManager").named(&["createQuery", "createNativeQuery"]),
    MethodMatcher::instance("javax.persistence.EntityManager").named(&["createQuery", "createNativeQuery"]),
    MethodMatcher::instance("org.hibernate.Session").named(&["createQuery", "createSQLQuery", "createNativeQuery"]),
];

/// Is the expression built from non-constant parts by concatenation or formatting?
fn dynamically_built<'t>(e: Node<'t>, cx: &Ctx<'_, 't>) -> bool {
    let e = e.unparen();
    match e.kind() {
        "binary_expression" => op(e) == "+" && cx.fm.constant_value(e).is_none(),
        "method_invocation" => {
            let constant = |a: &Node<'t>| a.unparen().kind() == "string_literal" || cx.fm.constant_value(*a).is_some();
            match cx.fm.call_name(e) {
                // String.format("...", constants...) and "...".formatted(constants...)
                Some("format" | "formatted" | "concat") => {
                    // `user.concat(";")`: the receiver is part of the result
                    let receiver_constant = e.field("object").is_none_or(|o| cx.fm.is_type_name(o) || constant(&o));
                    cx.fm.constant_value(e).is_none() && !(receiver_constant && cx.fm.args_of(e).iter().all(constant))
                }
                Some("toString") => {
                    let Some(o) = e.field("object").map(|o| o.unparen()) else { return false };
                    if !cx.type_of(o).class_name().is_some_and(|c| c.ends_with("StringBuilder") || c.ends_with("StringBuffer")) {
                        return false;
                    }
                    // a builder fed only constants builds a constant
                    let Some(i) = cx.fm.local_ref(o) else { return true };
                    let d = &cx.fm.binder.decls[i];
                    let init_dynamic = d.decl_node.field("value").is_some_and(|v| cx.fm.args_of(v.unparen()).iter().any(|a| !constant(a)));
                    let appends_dynamic = cx.fm.binder.uses[i].iter().any(|u| {
                        let mut cur = *u;
                        while let Some(p) = cur.parent().filter(|p| p.kind() == "method_invocation" && p.field("object") == Some(cur)) {
                            if matches!(cx.fm.call_name(p), Some("append" | "insert")) && cx.fm.args_of(p).iter().any(|a| !constant(a)) {
                                return true;
                            }
                            cur = p;
                        }
                        false
                    });
                    init_dynamic || appends_dynamic
                }
                _ => false,
            }
        }
        "identifier" => {
            // local built by concatenation: the last assignment before this use
            if let Some(i) = cx.fm.local_ref(e) {
                let d = &cx.fm.binder.decls[i];
                let last_assign = cx.fm.binder.uses[i]
                    .iter()
                    .filter(|u| u.start_byte() < e.start_byte())
                    .filter_map(|u| u.parent().filter(|p| p.kind() == "assignment_expression" && p.field("left").map(|l| l.unparen()) == Some(*u)))
                    .last();
                // the last assignment decides only if it surely runs before the use
                // (a statement of a block that encloses the use); otherwise any
                // dynamic value may reach it
                // `q += ";"` keeps the old value: never decisive on its own
                let unconditional = last_assign.is_some_and(|a| {
                    a.field("operator").is_some_and(|o| o.kind() == "=") && a.parent().filter(|p| p.kind() == "expression_statement").and_then(|st| st.parent()).is_some_and(|b| b.kind() == "block" && b.byte_range().contains(&e.start_byte()))
                });
                if let (true, Some(a)) = (unconditional, last_assign) {
                    return a.field("right").is_some_and(|v| dynamically_built(v, cx));
                }
                let init_dynamic = d.decl_node.field("value").is_some_and(|v| dynamically_built(v, cx));
                let any_assign_dynamic = cx.fm.binder.uses[i].iter().filter(|u| u.start_byte() < e.start_byte()).any(|u| {
                    u.parent().filter(|p| p.kind() == "assignment_expression" && p.field("left").map(|l| l.unparen()) == Some(*u)).and_then(|p| p.field("right")).is_some_and(|v| dynamically_built(v, cx))
                });
                return init_dynamic || any_assign_dynamic;
            }
            false
        }
        _ => false,
    }
}

declare_rule!(SqlInjection {
    id: "sql-injection",
    name: "SQL query built from dynamic strings",
    category: Security,
    severity: Warning,
    sources: ["sonar:S2077", "sonar:S3649", "spotbugs:SQL_NONCONSTANT_STRING_PASSED_TO_EXECUTE", "spotbugs:SQL_PREPARED_STATEMENT_GENERATED_FROM_NONCONSTANT_STRING", "spotbugs:SQL_INJECTION_JDBC"],
    kinds: ["method_invocation"],
    description: "Concatenating values into SQL lets attackers change the query (SQL injection). Use bind parameters (`?` with PreparedStatement, named parameters in JPA/JdbcTemplate).",
});

impl Rule for SqlInjection {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if !cx.fm.call_matches_any(n, SQL_SINKS) {
            return;
        }
        let Some(q) = cx.fm.args_of(n).first().copied() else { return };
        if dynamically_built(q, cx) {
            cx.report(q, "SQL built by string concatenation/formatting; use bind parameters");
        }
    }
}

const EXEC: MethodMatcher = MethodMatcher::instance("java.lang.Runtime").named(&["exec"]);

declare_rule!(CommandInjection {
    id: "command-injection",
    name: "OS command built from dynamic strings",
    category: Security,
    severity: Warning,
    sources: ["sonar:S2076", "sonar:S4721", "spotbugs:COMMAND_INJECTION"],
    kinds: ["method_invocation", "object_creation_expression"],
    description: "Building a shell command by concatenation allows injecting extra commands. Pass arguments as separate array elements and never through `sh -c`.",
});

impl Rule for CommandInjection {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let is_sink = if n.kind() == "method_invocation" { cx.fm.call_matches(n, &EXEC) } else { cx.type_of(n).is_class("java.lang.ProcessBuilder") };
        if !is_sink {
            return;
        }
        for a in cx.fm.args_of(n) {
            if dynamically_built(a, cx) {
                cx.report(a, "Command built by string concatenation; pass arguments separately");
                return;
            }
        }
    }
}

const XML_FACTORIES: &[(&str, &str)] = &[
    ("javax.xml.parsers.DocumentBuilderFactory", "newInstance"),
    ("javax.xml.parsers.SAXParserFactory", "newInstance"),
    ("javax.xml.stream.XMLInputFactory", "newInstance"),
    ("javax.xml.stream.XMLInputFactory", "newFactory"),
    ("javax.xml.transform.TransformerFactory", "newInstance"),
    ("javax.xml.validation.SchemaFactory", "newInstance"),
];

declare_rule!(XxeVulnerable {
    id: "xml-external-entities",
    name: "XML parser not protected against external entities",
    category: Security,
    severity: Warning,
    sources: ["sonar:S2755", "spotbugs:XXE_DOCUMENT", "spotbugs:XXE_SAXPARSER", "spotbugs:XXE_XMLSTREAMREADER"],
    kinds: ["method_invocation"],
    description: "XML factories resolve external entities by default, enabling file disclosure and SSRF (XXE). Disable DOCTYPEs or external entities (`setFeature(\"http://apache.org/xml/features/disallow-doctype-decl\", true)`, `XMLConstants.ACCESS_EXTERNAL_DTD` = \"\").",
});

impl Rule for XxeVulnerable {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(c) = cx.fm.select_method(n) else { return };
        if !XML_FACTORIES.iter().any(|(o, m)| *o == &*c.owner && *m == &*c.method.name) {
            return;
        }
        let Some(m) = cx.owning_callable(n).or_else(|| enclosing_method_decl(n)) else { return };
        let body = cx.text(m);
        if ["setFeature", "setAttribute", "setProperty", "setExpandEntityReferences", "setXIncludeAware", "ACCESS_EXTERNAL", "FEATURE_SECURE_PROCESSING", "SUPPORT_DTD", "IS_SUPPORTING_EXTERNAL_ENTITIES"].iter().any(|k| body.contains(k)) {
            return;
        }
        cx.report(n, format!("`{}` is used without disabling external entities", simple_name_of(&c.owner)));
    }
}

declare_rule!(CsrfDisabled {
    id: "csrf-protection-disabled",
    name: "CSRF protection disabled",
    category: Security,
    severity: Info,
    sources: ["sonar:S4502", "spotbugs:SPRING_CSRF_PROTECTION_DISABLED"],
    kinds: ["method_invocation"],
    description: "Disabling CSRF protection lets other sites submit authenticated requests on behalf of your users. Only disable it for stateless, token-authenticated APIs that never use cookies.",
});

impl Rule for CsrfDisabled {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let name = cx.fm.call_name(n).unwrap_or("");
        if name == "disable" {
            if let Some(o) = n.field("object").map(|o| o.unparen()) {
                if is_call_named(o, cx.src, &["csrf"]) && cx.fm.args_of(o).is_empty() {
                    cx.report(n, "CSRF protection is disabled");
                }
            }
        }
        if name == "csrf" {
            if let Some(a) = cx.fm.args_of(n).first() {
                let t = cx.text(*a);
                if t.contains(".disable()") || t.contains("::disable") {
                    cx.report(n, "CSRF protection is disabled");
                }
            }
        }
    }
}

declare_rule!(PermissiveCors {
    id: "permissive-cors",
    name: "CORS allows any origin",
    category: Security,
    severity: Warning,
    sources: ["sonar:S5122", "spotbugs:PERMISSIVE_CORS"],
    kinds: ["method_invocation", "annotation", "marker_annotation"],
    description: "Allowing every origin (`*`) lets any website read responses from your API in the user's browser. List the trusted origins explicitly.",
});

impl Rule for PermissiveCors {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if matches!(n.kind(), "annotation" | "marker_annotation") {
            let name = n.field("name").map(|x| cx.text(x)).unwrap_or("");
            if name.rsplit('.').next() != Some("CrossOrigin") {
                return;
            }
            let text = cx.text(n);
            if n.kind() == "marker_annotation" || !(text.contains("origins") || text.contains("value") || text.contains('"')) || text.contains("\"*\"") {
                cx.report(n, "@CrossOrigin without explicit origins allows every origin");
            }
            return;
        }
        let name = cx.fm.call_name(n).unwrap_or("");
        if !matches!(name, "allowedOrigins" | "addAllowedOrigin" | "setAllowedOrigins" | "allowedOriginPatterns" | "setAllowedOriginPatterns" | "addAllowedOriginPattern" | "setHeader" | "addHeader") {
            return;
        }
        let args = cx.fm.args_of(n);
        if matches!(name, "setHeader" | "addHeader") {
            if args.len() == 2 && cx.fm.string_value(args[0]).is_some_and(|h| h.eq_ignore_ascii_case("Access-Control-Allow-Origin")) && cx.fm.string_value(args[1]).as_deref() == Some("*") {
                cx.report(n, "Access-Control-Allow-Origin: * allows every origin");
            }
            return;
        }
        // "*" passed directly, or as an element of List.of / Arrays.asList / an array
        let star = |a: Node<'t>| cx.fm.string_value(a).as_deref() == Some("*");
        let any_star = args.iter().any(|a| {
            let a = a.unparen();
            star(a)
                || (a.kind() == "method_invocation" && matches!(cx.fm.call_name(a), Some("of" | "asList" | "singletonList")) && cx.fm.args_of(a).iter().any(|x| star(*x)))
                || (a.kind() == "array_creation_expression" && a.descendants().any(|x| x.kind() == "string_literal" && star(x)))
        });
        if any_star {
            cx.report(n, "CORS configuration allows every origin");
        }
    }
}

const KPG_INIT: MethodMatcher = MethodMatcher::instance("java.security.KeyPairGenerator").named(&["initialize"]);

declare_rule!(WeakRsaKey {
    id: "weak-key-size",
    name: "Cryptographic key too short",
    category: Security,
    severity: Error,
    sources: ["sonar:S4426", "spotbugs:RSA_KEY_SIZE", "spotbugs:BLOWFISH_KEY_SIZE"],
    kinds: ["method_invocation"],
    description: "RSA/DSA/DH keys shorter than 2048 bits and symmetric keys shorter than 128 bits can be broken. Use at least 2048 (RSA) / 256 (EC) / 128 (AES).",
});

impl Rule for WeakRsaKey {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let is_kpg = cx.fm.call_matches(n, &KPG_INIT);
        let is_kg = cx.fm.call_name(n) == Some("init") && n.field("object").is_some_and(|o| cx.type_of(o).is_class("javax.crypto.KeyGenerator"));
        if !is_kpg && !is_kg {
            return;
        }
        let Some(size) = cx.fm.args_of(n).first().and_then(|a| cx.fm.int_value(*a)) else { return };
        let min = if is_kpg { 2048 } else { 128 };
        // EC sizes are small by design
        if is_kpg && (224..600).contains(&size) {
            return;
        }
        if size < min {
            cx.report(n, format!("Key size {size} is too small (minimum {min})"));
        }
    }
}

declare_rule!(InsecureCookie {
    id: "insecure-cookie",
    name: "Cookie without Secure/HttpOnly",
    category: Security,
    severity: Warning,
    sources: ["sonar:S2092", "sonar:S3330", "spotbugs:INSECURE_COOKIE", "spotbugs:HTTPONLY_COOKIE"],
    kinds: ["object_creation_expression"],
    description: "Cookies without `Secure` travel over plain HTTP; without `HttpOnly` they are readable by injected scripts. Set both for session and auth cookies.",
});

impl Rule for InsecureCookie {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let t = cx.type_of(n);
        if !matches!(t.class_name(), Some("javax.servlet.http.Cookie" | "jakarta.servlet.http.Cookie")) {
            return;
        }
        let Some(m) = enclosing_method_decl(n) else { return };
        let body = cx.text(m);
        // a cookie sent by a (test) client request, not set on a response
        if (body.contains("setCookies(") || body.contains(".cookie(")) && !body.contains("addCookie(") {
            return;
        }
        // `setSecure(https)`: decided at runtime, not forgotten
        let set = |what: &str| body.contains(&format!("{what}(")) && !body.contains(&format!("{what}(false)"));
        if !set("setSecure") || !set("setHttpOnly") {
            cx.report(n, "Cookie is created without both setSecure(true) and setHttpOnly(true)");
        }
    }
}

declare_rule!(ObjectDeserialization {
    id: "unsafe-deserialization",
    name: "Java deserialization of untrusted data",
    category: Security,
    severity: Warning,
    enabled: false,
    sources: ["sonar:S5135", "spotbugs:OBJECT_DESERIALIZATION"],
    kinds: ["method_invocation"],
    description: "`ObjectInputStream.readObject()` on untrusted input can execute arbitrary code through gadget chains. Use a data format (JSON) or an `ObjectInputFilter` allow-list.",
});

impl Rule for ObjectDeserialization {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if cx.fm.call_name(n) == Some("readObject") && n.field("object").is_some_and(|o| is_subtype_any(cx.fm, &cx.type_of(o), &["java.io.ObjectInputStream"])) {
            if enclosing_method_decl(n).is_some_and(|m| method_name(m, cx.src) == "readObject") {
                return;
            }
            cx.report(n, "Deserialization with ObjectInputStream; add an ObjectInputFilter or avoid native serialization");
        }
    }
}

declare_rule!(StaticIv {
    id: "static-initialization-vector",
    name: "Constant initialization vector",
    category: Security,
    severity: Error,
    sources: ["sonar:S3329", "spotbugs:STATIC_IV"],
    kinds: ["object_creation_expression"],
    description: "Reusing a constant IV with CBC/GCM leaks plaintext relationships (and breaks GCM completely). Generate a random IV for every encryption with SecureRandom.",
});

impl Rule for StaticIv {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let t = cx.type_of(n);
        if !matches!(t.class_name(), Some("javax.crypto.spec.IvParameterSpec" | "javax.crypto.spec.GCMParameterSpec")) {
            return;
        }
        let args = cx.fm.args_of(n);
        let iv = if t.is_class("javax.crypto.spec.GCMParameterSpec") { args.get(1) } else { args.first() };
        let Some(iv) = iv.map(|a| a.unparen()) else { return };
        let constant = match iv.kind() {
            "array_creation_expression" => iv.descendants().any(|d| d.kind() == "array_initializer"),
            "method_invocation" => cx.fm.call_name(iv) == Some("getBytes") && iv.field("object").is_some_and(|o| cx.fm.constant_value(o).is_some()),
            "identifier" | "field_access" => cx.fm.field_ref(iv).is_some_and(|f| f.field.flags.is_static() && f.field.flags.is_final()),
            _ => false,
        };
        if constant {
            cx.report(iv, "Initialization vector is constant; generate a random IV per encryption");
        }
    }
}

declare_rule!(WorldWritablePermissions {
    id: "world-writable-permissions",
    name: "File permissions grant access to everyone",
    category: Security,
    severity: Warning,
    sources: ["sonar:S2612"],
    kinds: ["method_invocation"],
    description: "Granting write (or read) permission to all users (`rwxrwxrwx`, `setWritable(true, false)`) lets any local user tamper with the file.",
});

impl Rule for WorldWritablePermissions {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let name = cx.fm.call_name(n).unwrap_or("");
        let args = cx.fm.args_of(n);
        match name {
            "fromString" => {
                if let Some(p) = args.first().and_then(|a| cx.fm.string_value(*a)) {
                    if p.len() == 9 && &p[7..8] == "w" {
                        cx.report(n, format!("Permissions `{p}` make the file writable by everyone"));
                    }
                }
            }
            "setWritable" | "setReadable" | "setExecutable" => {
                if args.len() == 2 && cx.text(args[0]) == "true" && cx.text(args[1]) == "false" && n.field("object").is_some_and(|o| cx.type_of(o).is_class("java.io.File")) {
                    cx.report(n, format!("`{name}(true, false)` grants the permission to every user"));
                }
            }
            "add"
                if args.first().is_some_and(|a| cx.text(*a).ends_with("OTHERS_WRITE")) => {
                    cx.report(n, "Granting OTHERS_WRITE makes the file writable by everyone");
                }
            _ => {}
        }
    }
}

declare_rule!(InsecureTempFile {
    id: "insecure-temp-file",
    name: "Temporary file created insecurely",
    category: Security,
    severity: Info,
    sources: ["sonar:S5443", "spotbugs:PATH_TRAVERSAL_IN"],
    kinds: ["method_invocation", "object_creation_expression"],
    description: "`File.createTempFile` and hard-coded `/tmp` paths create files with default permissions in a shared directory. Use `Files.createTempFile` (restrictive permissions) or a private directory.",
});

impl Rule for InsecureTempFile {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        if n.kind() == "method_invocation" {
            if cx.fm.call_matches(n, &MethodMatcher::statics("java.io.File").named(&["createTempFile"])) {
                cx.report(n, "Use `Files.createTempFile` instead of `File.createTempFile`");
            }
            return;
        }
        if cx.type_of(n).is_class("java.io.File") {
            if let Some(a) = cx.fm.args_of(n).first() {
                if cx.fm.string_value(*a).is_some_and(|p| p.starts_with("/tmp/") || p.starts_with("/var/tmp/")) {
                    cx.report(n, "File in a shared temporary directory; use `Files.createTempFile`");
                }
            }
        }
    }
}

declare_rule!(CleartextUrl {
    id: "cleartext-http-url",
    name: "Clear-text HTTP URL",
    category: Security,
    severity: Info,
    scope: Main,
    enabled: false,
    sources: ["sonar:S5332"],
    kinds: ["string_literal"],
    description: "`http://` URLs send data unencrypted. Use https (local/loopback addresses are excluded).",
});

impl Rule for CleartextUrl {
    rule_meta!();
    fn check_node<'t>(&self, n: Node<'t>, cx: &mut Ctx<'_, 't>) {
        let Some(v) = grounds_syntax::string_literal_value(n, cx.src) else { return };
        if let Some(rest) = v.strip_prefix("http://") {
            let host = rest.split(['/', ':']).next().unwrap_or("");
            if host.is_empty() || host == "localhost" || host.starts_with("127.") || host.starts_with("0.0.0.0") || host.ends_with(".local") || host.contains("example") || host.starts_with("[::1]") || host.starts_with("www.w3.org") {
                return;
            }
            let _ = Ty::Unknown;
            cx.report(n, "Clear-text http:// URL; use https");
        }
    }
}
