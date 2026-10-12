//! One-off classpath dumps through the real build tool. Run rarely (when
//! dependencies change); the result in `.grounds/classpath.txt` is then read
//! on every load without invoking Gradle or Maven again.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

const GRADLE_INIT: &str = r#"
allprojects { p ->
    p.tasks.register("groundsClasspath") {
        doLast {
            def root = p.rootProject.projectDir
            def dir = new File(root, ".grounds/classpath")
            dir.mkdirs()
            def confs = new LinkedHashSet<String>(["compileClasspath", "testCompileClasspath", "annotationProcessor", "testAnnotationProcessor"])
            def sets = []
            def rel = root.toPath().relativize(p.projectDir.toPath()).toString()
            def ss = p.extensions.findByName("sourceSets")
            if (ss != null) {
                ss.each { s ->
                    confs << s.compileClasspathConfigurationName
                    confs << s.annotationProcessorConfigurationName
                    def runners = p.tasks.withType(Test).findAll { t ->
                        try { !t.testClassesDirs.files.disjoint(s.output.classesDirs.files) } catch (Throwable ignored) { false }
                    }.collect { it.name }
                    sets << [rel ?: ".", s.name, runners.join(",")].join("\t")
                }
            }
            def lines = new LinkedHashSet<String>()
            confs.each { n ->
                def c = p.configurations.findByName(n)
                if (c != null && c.canBeResolved) {
                    try { c.resolve().each { f -> lines << f.absolutePath } } catch (Throwable ignored) { }
                }
            }
            def name = p.path == ":" ? "root" : p.path.replace(':', '_')
            new File(dir, name + ".txt").text = lines.join("\n")
            new File(dir, name + ".sets").text = sets.join("\n")
        }
    }
}
"#;

fn merge(root: &Path, files: &[PathBuf]) -> Result<usize> {
    let mut all = std::collections::BTreeSet::new();
    for f in files {
        if let Ok(t) = std::fs::read_to_string(f) {
            for l in t.lines().flat_map(|l| l.split(':')) {
                let l = l.trim();
                if !l.is_empty() && Path::new(l).exists() {
                    all.insert(l.to_string());
                }
            }
        }
    }
    let out = root.join(".grounds/classpath.txt");
    std::fs::create_dir_all(out.parent().unwrap())?;
    std::fs::write(&out, all.iter().cloned().collect::<Vec<_>>().join("\n"))?;
    Ok(all.len())
}

pub fn gradle(root: &Path) -> Result<usize> {
    let gradlew = root.join("gradlew");
    let mut cmd = if gradlew.exists() { Command::new(gradlew) } else { Command::new("gradle") };
    let init = root.join(".grounds/grounds-classpath.init.gradle");
    std::fs::create_dir_all(init.parent().unwrap())?;
    let _ = std::fs::remove_dir_all(root.join(".grounds/classpath"));
    std::fs::write(&init, GRADLE_INIT)?;
    let status = cmd
        .current_dir(root)
        .args(["-q", "--no-configuration-cache", "--offline", "-I"])
        .arg(&init)
        .arg("groundsClasspath")
        .status()
        .or_else(|_| Command::new("gradle").current_dir(root).args(["-q", "-I"]).arg(&init).arg("groundsClasspath").status())
        .context("running gradle")?;
    if !status.success() {
        // retry online (dependencies not cached yet)
        let gradlew = root.join("gradlew");
        let mut cmd = if gradlew.exists() { Command::new(gradlew) } else { Command::new("gradle") };
        let st = cmd.current_dir(root).args(["-q", "--no-configuration-cache", "-I"]).arg(&init).arg("groundsClasspath").status()?;
        if !st.success() {
            bail!("gradle failed");
        }
    }
    let dir = root.join(".grounds/classpath");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir).map(|rd| rd.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    files.sort();
    // source set map: module dir, source set, test tasks running it
    let sets: Vec<String> = files.iter().filter(|f| f.extension().is_some_and(|x| x == "sets")).filter_map(|f| std::fs::read_to_string(f).ok()).filter(|t| !t.trim().is_empty()).collect();
    std::fs::write(root.join(".grounds/source-sets.txt"), sets.join("\n"))?;
    files.retain(|f| f.extension().is_some_and(|x| x == "txt"));
    merge(root, &files)
}

const MAVEN_CLASSPATH: [&str; 3] = ["dependency:build-classpath", "-Dmdep.outputFile=target/grounds-classpath.txt", "-Dmdep.includeScope=test"];

/// Through the lifecycle up to test-compile, compiling nothing: in a
/// multi-module build never installed, a module's dependency on a sibling
/// only resolves (to the sibling's output directory) once the reactor has run
/// the phase. Code generators bound to earlier phases still run; frontend
/// builds and checkers are skipped.
const MAVEN_REACTOR: &[&str] = &[
    "-Dmaven.main.skip=true", "-Dmaven.test.skip=true", "-Denforcer.skip=true", "-Dcheckstyle.skip=true", "-Dspotless.check.skip=true",
    "-Dspotless.apply.skip=true", "-Dlicense.skip=true", "-Drat.skip=true", "-Djacoco.skip=true", "-Dgpg.skip=true", "-Dskip.npm=true",
    "-Dskip.yarn=true", "-Dskip.pnpm=true", "-Dskip.installnodenpm=true", "-Dskip.installyarn=true", "-Dpmd.skip=true", "-Dspotbugs.skip=true",
    "-Dformatter.skip=true", "-Dimpsort.skip=true", "test-compile",
];

pub fn maven(root: &Path) -> Result<usize> {
    let mvn = || {
        let mvnw = root.join("mvnw");
        let mut cmd = if mvnw.exists() { Command::new(mvnw) } else { Command::new("mvn") };
        cmd.current_dir(root).args(["-q", "-B"]);
        cmd
    };
    let status = mvn().args(MAVEN_CLASSPATH).status().context("running maven")?;
    if !status.success() {
        eprintln!("grounds: retrying through the build lifecycle, without compiling (modules depending on modules not installed)");
        let status = mvn().args(MAVEN_REACTOR).args(MAVEN_CLASSPATH).status().context("running maven")?;
        if !status.success() {
            bail!("maven failed");
        }
    }
    let mut files = Vec::new();
    // target/ is usually gitignored, so walk without ignore filters
    let walker = ignore::WalkBuilder::new(root)
        .standard_filters(false)
        .max_depth(Some(8))
        .filter_entry(|e| !matches!(e.file_name().to_str(), Some(".git" | "node_modules" | "src" | ".grounds")))
        .build();
    for e in walker.flatten() {
        if e.file_name() == "grounds-classpath.txt" {
            files.push(e.path().to_path_buf());
        }
    }
    merge(root, &files)
}
