/*
 * javac as a judge: a long-lived process that attributes one Java source file
 * at a time with the JDK's own compiler (javax.tools, com.sun.source; module
 * jdk.compiler) and says what javac decided in it. Nothing beyond the JDK.
 *
 *   javac --add-exports jdk.compiler/com.sun.tools.javac.api=ALL-UNNAMED \
 *       -d OUT javac-helper/JavacHelper.java
 *   java --add-exports jdk.compiler/com.sun.tools.javac.api=ALL-UNNAMED \
 *       -Xss64m -cp OUT JavacHelper
 *
 * The internal classes it uses are com.sun.tools.javac.api.JavacTaskImpl and
 * JavacTaskPool (both in JDK 17 and 21), hence the --add-exports, the only
 * one needed (no --add-opens): the public JavacTask has analyze() but neither
 * enter() nor analyze(classes), and analyze() attributes every class in
 * javac's queue, which includes each class completed from the sourcepath
 * while attributing the file: their bodies too, and the classes those
 * load, until most of the project is attributed for one file.
 *
 * The protocol is one JSON object per line on stdin and stdout; nothing else
 * is written to stdout (the compiler's own output, and anything an annotation
 * processor prints, goes to stderr). A request:
 *
 *   {"id": 1, "sourcepath": [dir..], "classpath": [jar or dir..],
 *    "release": "17", "processorpath": [jar..] (optional),
 *    "roots": [file..] (optional), "files": [file..], "members": true,
 *    "reuse": false}
 *
 * For each file of `files` one answer line, then {"id": 1, "done": true}.
 * Each file is attributed alone: a task of its own parses it (with the files
 * of `roots`, which are entered so that annotation processors see them, but
 * not attributed), enters it and attributes only the classes it declares
 * (JavacTask.analyze(those classes)). Classes it refers to are completed from
 * the sourcepath and the classpath: their signatures read, their bodies not
 * attributed. Annotation processing is off unless `processorpath` is given.
 * Attribution goes on after errors (-XDshould-stop.ifError=FLOW), so that a
 * file with an unresolved import is still attributed as far as javac can.
 *
 * An answer:
 *
 *   {"id", "file", "ms": {"parse", "enter", "analyze", "scan", "total"},
 *    "diags": [{"code", "kind", "start", "end", "line", "msg"}],
 *    "other_diags": n (diagnostics in other files),
 *    "sites": [{"k": "call"|"ctor"|"field"|"mref"|"unres" (a name javac
 *               could not resolve), "start", "end",
 *               "ns", "ne" (the name), "line" (of the `(` for a call),
 *               "owner" (binary name of the declaring class), "name",
 *               "desc" (erased descriptor), "static",
 *               "err" (the symbol or a type in it is erroneous),
 *               "argerr" (the receiver or an argument is or contains an
 *                         erroneous expression),
 *               "hier" (a class whose members or subtyping decided the
 *                       answer has an erroneous supertype: missing),
 *               "missing" (that supertype, when known),
 *               "simp" (an unqualified name javac did not find that a static
 *                       import javac could not resolve may name),
 *               "srcs" (source files of the classes the member was looked
 *                       up in: the receiver's class and its supertypes, or
 *                       the enclosing classes' supertypes),
 *               "imp" (a type in the answer, the receiver or an argument
 *                      has the simple name of a single-type import javac
 *                      could not resolve, in this file or the declaring
 *                      class's: javac may have bound it to another class),
 *               "qual" (the class javac qualifies a call with in bytecode,
 *                       JLS 13.1: the receiver's erased type, or the
 *                       enclosing class the method is a member of),
 *               "anon" (a constructor of an anonymous class)}],
 *    "classes": [{"name", "start", "line", "end_line", "missing" (the first
 *                 missing supertype, "" when its hierarchy is complete),
 *                 "members": [{"name", "kind", "desc", "flags"}]}],
 *    "crash": "..." (when javac threw)}
 *
 * Offsets are UTF-8 byte offsets into the file (javac's are UTF-16 indexes;
 * they are converted), lines are 1-based. The compiler and the file manager
 * are made once and reused by every request. Each file needs a task of its
 * own (a JavacTask runs once), and by default the task's context, javac's
 * symbol table with it, is made anew for each file. With "reuse": true the
 * task comes from a com.sun.tools.javac.api.JavacTaskPool (what JShell
 * uses), which keeps the context between tasks with the same options: the
 * JDK's classes and every class completed from the sourcepath stay loaded,
 * and the attributed file is entered afresh. Measured, it is four times
 * faster and wrong: the pool's cleanup between tasks leaves the classes
 * completed from the sourcepath unusable (on 194 files of guava, spring-core
 * and lucene-core, 137 got another verdict, most with symbols javac could
 * no longer resolve). It is kept for that measurement only.
 *
 * With `processorpath`, the processors are loaded once per path and each
 * task gets new instances of them (task.setProcessors): javac would load
 * them in a new class loader for every task. Lombok works so in one process
 * across tasks (it patches javac per context). javac runs processors only
 * on the files of the task, not on classes it completes from the
 * sourcepath, so members Lombok generates in other files are seen only when
 * those files are among the `roots`.
 *
 * javac's own error recovery is not a judge everywhere: on classes whose
 * supertypes are missing it can pick another overload for an erroneous
 * argument (marked "argerr"), resolve in a class whose missing supertype
 * would have had a closer member ("hier"), bind a simple name whose import
 * failed to another class of that name ("imp"), and it can throw (an
 * NullPointerException in Types.sideCast for an instanceof whose type has a
 * missing supertype; a module-info.java on the sourcepath without its
 * required modules makes packages with a package-info.java inaccessible).
 */

import com.sun.source.tree.*;
import com.sun.source.util.*;
import com.sun.tools.javac.api.JavacTaskImpl;
import com.sun.tools.javac.api.JavacTaskPool;

import javax.annotation.processing.Processor;
import javax.lang.model.element.*;
import javax.lang.model.type.*;
import javax.lang.model.util.Elements;
import javax.lang.model.util.Types;
import javax.tools.*;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.util.*;

public final class JavacHelper {
    private static final JavaCompiler COMPILER = ToolProvider.getSystemJavaCompiler();
    private static StandardJavaFileManager fileManager;
    private static String configured = "";
    /**
     * The annotation processors of each processor path, loaded once: javac
     * would make a class loader for the path, and load the processors anew,
     * for every task (for Lombok, half a second a file). Each task gets new
     * instances of the classes loaded once.
     */
    private static final Map<String, ClassLoader> PROCESSOR_LOADERS = new HashMap<>();

    static List<Processor> processors(List<String> path) throws IOException {
        ClassLoader loader = PROCESSOR_LOADERS.get(path.toString());
        if (loader == null) {
            List<java.net.URL> urls = new ArrayList<>();
            for (String p : path) urls.add(new File(p).toURI().toURL());
            loader = new java.net.URLClassLoader(urls.toArray(new java.net.URL[0]), JavacHelper.class.getClassLoader());
            PROCESSOR_LOADERS.put(path.toString(), loader);
        }
        List<Processor> r = new ArrayList<>();
        for (Processor p : ServiceLoader.load(Processor.class, loader)) r.add(p);
        return r;
    }

    /** Contexts kept between tasks with the same options ("reuse": true). */
    private static final JavacTaskPool POOL = new JavacTaskPool(1);

    public static void main(String[] args) throws Exception {
        PrintStream out = new PrintStream(new FileOutputStream(FileDescriptor.out), false, "UTF-8");
        // anything else that writes to System.out (a processor) goes to stderr
        System.setOut(System.err);
        BufferedReader in = new BufferedReader(new InputStreamReader(System.in, StandardCharsets.UTF_8));
        fileManager = COMPILER.getStandardFileManager(null, Locale.ROOT, StandardCharsets.UTF_8);
        String line;
        while ((line = in.readLine()) != null) {
            if (line.isBlank()) continue;
            Object id = null;
            try {
                @SuppressWarnings("unchecked")
                Map<String, Object> req = (Map<String, Object>) new Json(line).value();
                id = req.get("id");
                handle(req, id, out);
            } catch (Throwable t) {
                StringBuilder sb = new StringBuilder("{\"id\":");
                Json.write(sb, id);
                sb.append(",\"error\":");
                Json.write(sb, String.valueOf(t));
                sb.append(",\"done\":true}");
                out.println(sb);
            }
            out.flush();
        }
    }

    @SuppressWarnings("unchecked")
    private static List<String> strings(Object o) {
        if (o == null) return List.of();
        List<String> r = new ArrayList<>();
        for (Object x : (List<Object>) o) r.add(String.valueOf(x));
        return r;
    }

    private static List<File> files(List<String> s) {
        List<File> r = new ArrayList<>();
        for (String x : s) r.add(new File(x));
        return r;
    }

    private static void handle(Map<String, Object> req, Object id, PrintStream out) throws IOException {
        List<String> sourcepath = strings(req.get("sourcepath"));
        List<String> classpath = strings(req.get("classpath"));
        List<String> processorpath = req.get("processorpath") == null ? null : strings(req.get("processorpath"));
        String release = req.get("release") == null ? null : String.valueOf(req.get("release"));
        boolean members = !Boolean.FALSE.equals(req.get("members"));
        boolean reuse = Boolean.TRUE.equals(req.get("reuse"));
        // set the locations only when they change: the file manager keeps
        // what it opened (jars, directory listings) between tasks
        String config = sourcepath + "|" + classpath + "|" + processorpath;
        if (!config.equals(configured)) {
            fileManager.setLocation(StandardLocation.SOURCE_PATH, files(sourcepath));
            fileManager.setLocation(StandardLocation.CLASS_PATH, files(classpath));
            if (processorpath != null) fileManager.setLocation(StandardLocation.ANNOTATION_PROCESSOR_PATH, files(processorpath));
            configured = config;
        }
        List<String> options = new ArrayList<>(List.of(
                "-XDshould-stop.ifError=FLOW", "-XDshould-stop.ifNoError=FLOW",
                "-Xmaxerrs", "100000", "-nowarn", "-Xlint:none", "-encoding", "UTF-8", "-implicit:none"));
        if (release != null) options.addAll(List.of("--release", release));
        if (processorpath == null) options.add("-proc:none");
        List<File> roots = files(strings(req.get("roots")));
        for (String f : strings(req.get("files"))) {
            StringBuilder sb = new StringBuilder();
            FileJob job = new FileJob(f, options, roots, members, reuse ? POOL : null);
            if (processorpath != null) job.processors = processors(processorpath);
            job.run(sb, id);
            out.println(sb);
            out.flush();
        }
        out.println("{\"id\":" + Json.str(id) + ",\"done\":true}");
    }

    /** One file: a task of its own, and the answer. */
    private static final class FileJob {
        final String path;
        final List<String> options;
        final List<File> roots;
        final boolean members;
        Trees trees;
        Types types;
        Elements elements;
        SourcePositions pos;
        CompilationUnitTree cu;
        LineMap lines;
        CharSequence text;
        int[] byteAt; // UTF-16 index -> UTF-8 offset; null when the file is ASCII
        final Map<TypeElement, String> incomplete = new HashMap<>();

        final JavacTaskPool pool;

        FileJob(String path, List<String> options, List<File> roots, boolean members, JavacTaskPool pool) {
            this.pool = pool;
            this.path = path;
            this.options = options;
            this.roots = roots;
            this.members = members;
        }

        long tParse, tEnter, tAnalyze, tScan;

        List<Processor> processors;

        void attribute(JavacTaskImpl task, JavaFileObject me, List<Map<String, Object>> sites, List<Map<String, Object>> classes, long t0) throws IOException {
            if (processors != null) task.setProcessors(processors);
            trees = Trees.instance(task);
            types = task.getTypes();
            elements = task.getElements();
            pos = trees.getSourcePositions();
            for (CompilationUnitTree u : task.parse()) {
                if (u.getSourceFile() == me) cu = u;
            }
            tParse = System.nanoTime() - t0;
            long t1 = System.nanoTime();
            Iterable<? extends Element> entered = task.enter();
            tEnter = System.nanoTime() - t1;
            t1 = System.nanoTime();
            // only the classes this file declares
            List<Element> mine = new ArrayList<>();
            for (Element e : entered) {
                TreePath p = trees.getPath(e);
                if (p != null && p.getCompilationUnit() == cu) mine.add(e);
            }
            if (!mine.isEmpty()) task.analyze(mine);
            tAnalyze = System.nanoTime() - t1;
            t1 = System.nanoTime();
            text = cu.getSourceFile().getCharContent(true);
            lines = cu.getLineMap();
            byteAt = utf8Offsets(text);
            new Scanner(sites, classes).scan(new TreePath(cu), null);
            tScan = System.nanoTime() - t1;
        }

        void run(StringBuilder sb, Object id) {
            sb.append("{\"id\":").append(Json.str(id)).append(",\"file\":");
            Json.write(sb, path);
            long t0 = System.nanoTime();
            DiagnosticCollector<JavaFileObject> dc = new DiagnosticCollector<>();
            String crash = null;
            List<Map<String, Object>> sites = new ArrayList<>();
            List<Map<String, Object>> classes = new ArrayList<>();
            JavaFileObject me = null;
            try {
                me = fileManager.getJavaFileObjects(new File(path)).iterator().next();
                List<JavaFileObject> units = new ArrayList<>();
                units.add(me);
                for (File r : roots) {
                    if (!r.getCanonicalFile().equals(new File(path).getCanonicalFile())) {
                        units.add(fileManager.getJavaFileObjects(r).iterator().next());
                    }
                }
                PrintWriter err = new PrintWriter(new OutputStreamWriter(System.err, StandardCharsets.UTF_8));
                if (pool != null) {
                    final JavaFileObject self = me;
                    Throwable[] thrown = new Throwable[1];
                    pool.getTask(err, fileManager, dc, options, null, units, task -> {
                        try {
                            attribute((JavacTaskImpl) task, self, sites, classes, t0);
                        } catch (Throwable t) {
                            thrown[0] = t;
                        }
                        return null;
                    });
                    if (thrown[0] != null) throw thrown[0];
                } else {
                    attribute((JavacTaskImpl) COMPILER.getTask(err, fileManager, dc, options, null, units), me, sites, classes, t0);
                }
            } catch (Throwable t) {
                StringWriter w = new StringWriter();
                t.printStackTrace(new PrintWriter(w));
                String s = w.toString();
                crash = s.length() > 2000 ? s.substring(0, 2000) : s;
            }
            long total = System.nanoTime() - t0;
            sb.append(",\"ms\":{\"parse\":").append(tParse / 1e6).append(",\"enter\":").append(tEnter / 1e6).append(",\"analyze\":").append(tAnalyze / 1e6).append(",\"scan\":").append(tScan / 1e6).append(",\"total\":").append(total / 1e6).append('}');
            // diagnostics of this file; the others are only counted
            sb.append(",\"diags\":[");
            int other = 0;
            boolean first = true;
            for (Diagnostic<? extends JavaFileObject> d : dc.getDiagnostics()) {
                if (d.getSource() == null || me == null || !d.getSource().toUri().equals(me.toUri())) {
                    other++;
                    continue;
                }
                if (!first) sb.append(',');
                first = false;
                Map<String, Object> m = new LinkedHashMap<>();
                m.put("code", d.getCode());
                m.put("kind", d.getKind().toString());
                m.put("start", off(d.getStartPosition()));
                m.put("end", off(d.getEndPosition()));
                m.put("line", d.getLineNumber());
                String msg = d.getMessage(Locale.ROOT).replace('\n', ' ').replaceAll(" +", " ");
                m.put("msg", msg.length() > 400 ? msg.substring(0, 400) : msg);
                Json.write(sb, m);
            }
            sb.append("],\"other_diags\":").append(other);
            sb.append(",\"sites\":");
            Json.write(sb, sites);
            if (members) {
                sb.append(",\"classes\":");
                Json.write(sb, classes);
            }
            if (crash != null) {
                sb.append(",\"crash\":");
                Json.write(sb, crash);
            }
            sb.append('}');
        }

        long off(long charPos) {
            if (charPos < 0) return -1;
            if (byteAt == null || text == null) return charPos;
            return byteAt[(int) Math.min(charPos, byteAt.length - 1)];
        }

        static int[] utf8Offsets(CharSequence s) {
            boolean ascii = true;
            for (int i = 0; i < s.length() && ascii; i++) ascii = s.charAt(i) < 0x80;
            if (ascii) return null;
            int[] r = new int[s.length() + 1];
            int b = 0;
            for (int i = 0; i < s.length(); i++) {
                r[i] = b;
                char c = s.charAt(i);
                if (c < 0x80) b += 1;
                else if (c < 0x800) b += 2;
                else if (Character.isHighSurrogate(c)) b += 2; // with its low surrogate: 4
                else if (Character.isLowSurrogate(c)) b += 2;
                else b += 3;
            }
            r[s.length()] = b;
            return r;
        }

        /** Whether a type, or a type in it, is erroneous. */
        boolean erroneous(TypeMirror t) {
            return erroneous(t, 0);
        }

        boolean erroneous(TypeMirror t, int depth) {
            if (t == null) return true;
            if (depth > 12) return false;
            switch (t.getKind()) {
                case ERROR:
                    return true;
                case ARRAY:
                    return erroneous(((ArrayType) t).getComponentType(), depth + 1);
                case DECLARED:
                    for (TypeMirror a : ((DeclaredType) t).getTypeArguments()) if (erroneous(a, depth + 1)) return true;
                    return false;
                case EXECUTABLE: {
                    ExecutableType e = (ExecutableType) t;
                    if (erroneous(e.getReturnType(), depth + 1)) return true;
                    for (TypeMirror p : e.getParameterTypes()) if (erroneous(p, depth + 1)) return true;
                    return false;
                }
                case WILDCARD: {
                    WildcardType w = (WildcardType) t;
                    return (w.getExtendsBound() != null && erroneous(w.getExtendsBound(), depth + 1)) || (w.getSuperBound() != null && erroneous(w.getSuperBound(), depth + 1));
                }
                case INTERSECTION:
                    for (TypeMirror b : ((IntersectionType) t).getBounds()) if (erroneous(b, depth + 1)) return true;
                    return false;
                case UNION:
                    for (TypeMirror b : ((UnionType) t).getAlternatives()) if (erroneous(b, depth + 1)) return true;
                    return false;
                default:
                    return false;
            }
        }

        final Map<CompilationUnitTree, Set<String>> failedImports = new HashMap<>();

        final Map<CompilationUnitTree, Set<String>> failedStatic = new HashMap<>();

        /** The member names of a file's static imports whose class javac could not resolve ("*" for an on-demand one). */
        Set<String> failedStaticImports(CompilationUnitTree u) {
            Set<String> known = failedStatic.get(u);
            if (known != null) return known;
            Set<String> r = new HashSet<>();
            for (ImportTree it : u.getImports()) {
                if (!it.isStatic() || !(it.getQualifiedIdentifier() instanceof MemberSelectTree)) continue;
                MemberSelectTree q = (MemberSelectTree) it.getQualifiedIdentifier();
                TreePath qp = new TreePath(new TreePath(new TreePath(u), it), q);
                Element owner = trees.getElement(new TreePath(qp, q.getExpression()));
                if (owner == null || owner.asType() == null || owner.asType().getKind() == TypeKind.ERROR) r.add(q.getIdentifier().toString());
            }
            failedStatic.put(u, r);
            return r;
        }

        /** The simple names of the single-type imports of a file that javac could not resolve. */
        Set<String> failedImports(CompilationUnitTree u) {
            Set<String> known = failedImports.get(u);
            if (known != null) return known;
            Set<String> r = new HashSet<>();
            for (ImportTree it : u.getImports()) {
                if (it.isStatic() || !(it.getQualifiedIdentifier() instanceof MemberSelectTree)) continue;
                MemberSelectTree q = (MemberSelectTree) it.getQualifiedIdentifier();
                if (q.getIdentifier().contentEquals("*")) continue;
                Element e = trees.getElement(new TreePath(new TreePath(new TreePath(u), it), q));
                if (e == null || e.asType() == null || e.asType().getKind() == TypeKind.ERROR) r.add(q.getIdentifier().toString());
            }
            failedImports.put(u, r);
            return r;
        }

        /** Whether a class has an erroneous supertype, directly or further up. */
        boolean incompleteClass(TypeElement c, int depth) {
            return !missingSuper(c, depth).isEmpty();
        }

        /** The first erroneous (missing) supertype of a class, directly or further up; "" when none. */
        String missingSuper(TypeElement c, int depth) {
            if (c == null || depth > 40) return "";
            String known = incomplete.get(c);
            if (known != null) return known;
            incomplete.put(c, ""); // cycles
            String r = "";
            List<TypeMirror> sups = new ArrayList<>(c.getInterfaces());
            sups.add(c.getSuperclass());
            for (TypeMirror s : sups) {
                if (s.getKind() == TypeKind.ERROR) r = s.toString() + " (of " + c.getQualifiedName() + ")";
                else if (s.getKind() == TypeKind.DECLARED) r = missingSuper((TypeElement) ((DeclaredType) s).asElement(), depth + 1);
                if (!r.isEmpty()) break;
            }
            incomplete.put(c, r);
            return r;
        }

        /** The source files (not class files) of a class and its supertypes. */
        void sourcesUp(TypeElement c, Set<String> out, int depth) {
            if (c == null || depth > 8 || out.size() > 8) return;
            TreePath p = trees.getPath(c);
            if (p != null && p.getCompilationUnit() != cu) out.add(p.getCompilationUnit().getSourceFile().toUri().getPath());
            TypeMirror sup = c.getSuperclass();
            if (sup.getKind() == TypeKind.DECLARED) sourcesUp((TypeElement) ((DeclaredType) sup).asElement(), out, depth + 1);
            for (TypeMirror i : c.getInterfaces())
                if (i.getKind() == TypeKind.DECLARED) sourcesUp((TypeElement) ((DeclaredType) i).asElement(), out, depth + 1);
        }

        /** Whether the answer for this type may differ with its missing supertypes. */
        boolean incompleteType(TypeMirror t) {
            if (t == null) return false;
            t = erasureOrSelf(t);
            if (t.getKind() == TypeKind.ARRAY) return incompleteType(((ArrayType) t).getComponentType());
            if (t.getKind() == TypeKind.TYPEVAR) return incompleteType(((TypeVariable) t).getUpperBound());
            if (t.getKind() == TypeKind.INTERSECTION) {
                for (TypeMirror b : ((IntersectionType) t).getBounds()) if (incompleteType(b)) return true;
                return false;
            }
            if (t.getKind() != TypeKind.DECLARED) return false;
            return incompleteClass((TypeElement) ((DeclaredType) t).asElement(), 0);
        }

        TypeMirror erasureOrSelf(TypeMirror t) {
            try {
                return types.erasure(t);
            } catch (RuntimeException e) {
                return t;
            }
        }

        String binaryName(Element e) {
            if (e instanceof TypeElement) {
                try {
                    return elements.getBinaryName((TypeElement) e).toString();
                } catch (RuntimeException x) {
                    return "!" + e;
                }
            }
            return e == null ? "?" : "?" + e.getKind() + ":" + e;
        }

        String desc(TypeMirror t) {
            if (t == null) return "!null";
            switch (t.getKind()) {
                case BOOLEAN: return "Z";
                case BYTE: return "B";
                case CHAR: return "C";
                case SHORT: return "S";
                case INT: return "I";
                case LONG: return "J";
                case FLOAT: return "F";
                case DOUBLE: return "D";
                case VOID: return "V";
                case ARRAY: return "[" + desc(((ArrayType) t).getComponentType());
                case DECLARED: return "L" + binaryName(((DeclaredType) t).asElement()).replace('.', '/') + ";";
                case TYPEVAR:
                case INTERSECTION: {
                    TypeMirror e = erasureOrSelf(t);
                    return e == t ? "?" + t.getKind() : desc(e);
                }
                case ERROR: return "!" + t;
                default: return "?" + t.getKind();
            }
        }

        /** The erased descriptor of a method, constructor or field symbol. */
        String descOf(Element e) {
            TypeMirror t = erasureOrSelf(e.asType());
            if (t instanceof ExecutableType) {
                ExecutableType x = (ExecutableType) t;
                StringBuilder b = new StringBuilder("(");
                for (TypeMirror p : x.getParameterTypes()) b.append(desc(erasureOrSelf(p)));
                return b.append(')').append(desc(erasureOrSelf(x.getReturnType()))).toString();
            }
            return desc(t);
        }

        int line(long charPos) {
            return charPos < 0 ? -1 : (int) lines.getLineNumber(charPos);
        }

        final class Scanner extends TreePathScanner<Void, Void> {
            final List<Map<String, Object>> sites;
            final List<Map<String, Object>> classes;

            Scanner(List<Map<String, Object>> sites, List<Map<String, Object>> classes) {
                this.sites = sites;
                this.classes = classes;
            }

            long start(Tree t) {
                return pos.getStartPosition(cu, t);
            }

            long end(Tree t) {
                return pos.getEndPosition(cu, t);
            }

            Map<String, Object> site(String kind, Tree node, long nameStart, long nameEnd, Element sym) {
                Map<String, Object> m = new LinkedHashMap<>();
                m.put("k", kind);
                m.put("start", off(start(node)));
                m.put("end", off(end(node)));
                m.put("ns", off(nameStart));
                m.put("ne", off(nameEnd));
                m.put("line", line(nameStart));
                boolean err = sym == null || sym.asType() == null || sym.asType().getKind() == TypeKind.ERROR;
                if (sym != null && !err) {
                    Element owner = sym.getEnclosingElement();
                    m.put("owner", binaryName(owner));
                    m.put("name", sym.getSimpleName().toString());
                    m.put("desc", descOf(sym));
                    m.put("static", sym.getModifiers().contains(Modifier.STATIC));
                    err = erroneous(sym.asType()) || owner == null || owner.asType().getKind() == TypeKind.ERROR || String.valueOf(m.get("desc")).contains("!");
                    boolean isMember = sym.getKind() == ElementKind.METHOD || sym.getKind() == ElementKind.CONSTRUCTOR || sym.getKind() == ElementKind.FIELD || sym.getKind() == ElementKind.ENUM_CONSTANT;
                    if (!isMember) m.put("symkind", sym.getKind().toString());
                } else if (sym != null) {
                    m.put("name", sym.getSimpleName().toString());
                }
                m.put("err", err);
                sites.add(m);
                return m;
            }

            /** The erroneous types among the receiver and the arguments, and missing supertypes. */
            void argumentChecks(Map<String, Object> m, TreePath at, ExpressionTree receiver, List<? extends ExpressionTree> args, Element sym) {
                boolean argerr = false, hier = false;
                if (receiver != null) {
                    TypeMirror rt = trees.getTypeMirror(new TreePath(at, receiver));
                    if (rt != null && rt.getKind() != TypeKind.PACKAGE) {
                        argerr |= erroneous(rt) || containsError(new TreePath(at, receiver));
                        hier |= incompleteType(rt);
                    }
                }
                for (ExpressionTree a : args) {
                    // anything erroneous inside an argument: a lambda whose
                    // body does not attribute still fits a functional
                    // interface, and a generic call's inference may settle on
                    // Object, so the argument's own type can be wrong without
                    // being erroneous, and javac then picks another overload
                    argerr |= containsError(new TreePath(at, a));
                    TypeMirror t = trees.getTypeMirror(new TreePath(at, a));
                    if (t == null) continue; // a lambda or method reference without a target
                    argerr |= erroneous(t);
                    hier |= incompleteType(t);
                }
                // the class the member was found in, and the classes it was looked up from
                if (sym != null && sym.getEnclosingElement() instanceof TypeElement) {
                    hier |= incompleteClass((TypeElement) sym.getEnclosingElement(), 0);
                }
                // an unqualified name: the enclosing classes javac looked in,
                // from the innermost to the one it was found in
                if (receiver == null && sym != null && sym.getEnclosingElement() instanceof TypeElement) {
                    TypeMirror owner = erasureOrSelf(sym.getEnclosingElement().asType());
                    for (TreePath p = at; p != null; p = p.getParentPath()) {
                        if (p.getLeaf() instanceof ClassTree) {
                            Element c = trees.getElement(p);
                            if (!(c instanceof TypeElement)) continue;
                            hier |= incompleteClass((TypeElement) c, 0);
                            if (types.isSubtype(erasureOrSelf(c.asType()), owner)) break;
                        }
                    }
                } else if (receiver == null) {
                    // not found: every enclosing class was looked in
                    for (TreePath p = at; p != null; p = p.getParentPath()) {
                        if (p.getLeaf() instanceof ClassTree) {
                            Element c = trees.getElement(p);
                            if (c instanceof TypeElement && incompleteClass((TypeElement) c, 0)) {
                                hier = true;
                                m.put("missing", missingSuper((TypeElement) c, 0));
                            }
                        }
                    }
                }
                // the source files of the classes the member was looked up in
                // (the receiver's class and its supertypes, or the enclosing
                // classes' supertypes): a copy of a class in test data or a
                // fake is as good as the real one to javac on a sourcepath
                Set<String> srcs = new LinkedHashSet<>();
                if (receiver != null) {
                    TypeMirror rt = erasureOrSelf(trees.getTypeMirror(new TreePath(at, receiver)));
                    if (rt != null && rt.getKind() == TypeKind.DECLARED) sourcesUp((TypeElement) ((DeclaredType) rt).asElement(), srcs, 0);
                } else {
                    for (TreePath p = at; p != null; p = p.getParentPath()) {
                        if (p.getLeaf() instanceof ClassTree) {
                            Element c = trees.getElement(p);
                            if (c instanceof TypeElement) {
                                TypeMirror sup = ((TypeElement) c).getSuperclass();
                                if (sup.getKind() == TypeKind.DECLARED) sourcesUp((TypeElement) ((DeclaredType) sup).asElement(), srcs, 0);
                                for (TypeMirror i : ((TypeElement) c).getInterfaces())
                                    if (i.getKind() == TypeKind.DECLARED) sourcesUp((TypeElement) ((DeclaredType) i).asElement(), srcs, 0);
                            }
                        }
                    }
                }
                if (!srcs.isEmpty()) m.put("srcs", new ArrayList<>(srcs));
                // an unqualified name javac did not find, which a static
                // import javac could not resolve may have brought in
                if (receiver == null && (sym == null || sym.asType() == null || sym.asType().getKind() == TypeKind.ERROR)) {
                    Tree leaf = at.getLeaf();
                    String name = leaf instanceof MethodInvocationTree && ((MethodInvocationTree) leaf).getMethodSelect() instanceof IdentifierTree
                            ? ((IdentifierTree) ((MethodInvocationTree) leaf).getMethodSelect()).getName().toString()
                            : leaf instanceof IdentifierTree ? ((IdentifierTree) leaf).getName().toString() : null;
                    Set<String> failed = failedStaticImports(cu);
                    if (name != null && (failed.contains(name) || failed.contains("*"))) m.put("simp", true);
                }
                if (receiver != null && hier && !m.containsKey("missing")) {
                    TypeMirror rt = erasureOrSelf(trees.getTypeMirror(new TreePath(at, receiver)));
                    if (rt != null && rt.getKind() == TypeKind.DECLARED) m.put("missing", missingSuper((TypeElement) ((DeclaredType) rt).asElement(), 0));
                }
                m.put("argerr", argerr);
                m.put("hier", hier);
                // a name a failed single-type import should have bound: javac
                // then looks further (the package, the on-demand imports) and
                // may find another class of that name, with no error
                Set<String> names = new HashSet<>();
                Set<TypeElement> classes = new HashSet<>();
                if (sym != null) {
                    simpleNames(sym.asType(), names, classes, 0);
                    if (sym.getEnclosingElement() instanceof TypeElement) classes.add((TypeElement) sym.getEnclosingElement());
                }
                if (receiver != null) simpleNames(trees.getTypeMirror(new TreePath(at, receiver)), names, classes, 0);
                for (ExpressionTree a : args) simpleNames(trees.getTypeMirror(new TreePath(at, a)), names, classes, 0);
                Set<String> failed = new HashSet<>(failedImports(cu));
                for (TypeElement c : classes) {
                    TreePath p = trees.getPath(c);
                    if (p != null && p.getCompilationUnit() != cu) failed.addAll(failedImports(p.getCompilationUnit()));
                }
                names.retainAll(failed);
                m.put("imp", !names.isEmpty());
            }

            void simpleNames(TypeMirror t, Set<String> names, Set<TypeElement> classes, int depth) {
                if (t == null || depth > 6) return;
                switch (t.getKind()) {
                    case DECLARED: {
                        TypeElement e = (TypeElement) ((DeclaredType) t).asElement();
                        names.add(e.getSimpleName().toString());
                        classes.add(e);
                        for (TypeMirror a : ((DeclaredType) t).getTypeArguments()) simpleNames(a, names, classes, depth + 1);
                        break;
                    }
                    case ERROR:
                        names.add(t.toString().replaceAll(".*[.]", ""));
                        break;
                    case ARRAY:
                        simpleNames(((ArrayType) t).getComponentType(), names, classes, depth + 1);
                        break;
                    case EXECUTABLE: {
                        ExecutableType x = (ExecutableType) t;
                        simpleNames(x.getReturnType(), names, classes, depth + 1);
                        for (TypeMirror p : x.getParameterTypes()) simpleNames(p, names, classes, depth + 1);
                        break;
                    }
                    case TYPEVAR:
                        simpleNames(((TypeVariable) t).getUpperBound(), names, classes, depth + 1);
                        break;
                    case WILDCARD: {
                        WildcardType w = (WildcardType) t;
                        simpleNames(w.getExtendsBound(), names, classes, depth + 1);
                        simpleNames(w.getSuperBound(), names, classes, depth + 1);
                        break;
                    }
                    case INTERSECTION:
                        for (TypeMirror b : ((IntersectionType) t).getBounds()) simpleNames(b, names, classes, depth + 1);
                        break;
                    default:
                }
            }

            /** Whether an expression or anything in it is erroneous: a type or a name javac could not resolve. */
            boolean containsError(TreePath expr) {
                boolean[] found = new boolean[1];
                new TreePathScanner<Void, Void>() {
                    @Override
                    public Void scan(Tree t, Void v) {
                        if (found[0] || t == null) return null;
                        if (t instanceof ExpressionTree) {
                            TreePath p = new TreePath(getCurrentPath(), t);
                            TypeMirror tm = trees.getTypeMirror(p);
                            if (tm != null && tm.getKind() == TypeKind.ERROR) {
                                found[0] = true;
                                return null;
                            }
                        }
                        return super.scan(t, v);
                    }
                }.scan(expr, null);
                return found[0];
            }

            /** The class javac names in the invoke instruction (JLS 13.1). */
            String qualifier(TreePath at, ExpressionTree receiver, Element sym) {
                if (receiver != null) {
                    TypeMirror rt = trees.getTypeMirror(new TreePath(at, receiver));
                    if (rt == null) return null;
                    rt = erasureOrSelf(rt);
                    if (rt.getKind() == TypeKind.DECLARED) return binaryName(((DeclaredType) rt).asElement());
                    if (rt.getKind() == TypeKind.ARRAY) return desc(rt);
                    return null;
                }
                if (sym == null || !(sym.getEnclosingElement() instanceof TypeElement)) return null;
                TypeMirror owner = erasureOrSelf(sym.getEnclosingElement().asType());
                for (TreePath p = at; p != null; p = p.getParentPath()) {
                    if (p.getLeaf() instanceof ClassTree) {
                        Element c = trees.getElement(p);
                        if (c instanceof TypeElement && types.isSubtype(erasureOrSelf(c.asType()), owner)) return binaryName(c);
                    }
                }
                return null;
            }

            /** The offset of the `(` after a method name: javac's position of a call. */
            long paren(long from) {
                int i = (int) from;
                while (i < text.length()) {
                    char c = text.charAt(i);
                    if (c == '(') return i;
                    if (Character.isWhitespace(c)) i++;
                    else if (c == '/' && i + 1 < text.length() && text.charAt(i + 1) == '/') {
                        while (i < text.length() && text.charAt(i) != '\n') i++;
                    } else if (c == '/' && i + 1 < text.length() && text.charAt(i + 1) == '*') {
                        int e = text.toString().indexOf("*/", i + 2);
                        i = e < 0 ? text.length() : e + 2;
                    } else return from;
                }
                return from;
            }

            @Override
            public Void visitMethodInvocation(MethodInvocationTree t, Void v) {
                TreePath here = getCurrentPath();
                ExpressionTree sel = t.getMethodSelect();
                Element sym = trees.getElement(new TreePath(here, sel));
                long ne = end(sel), ns;
                ExpressionTree receiver = null;
                String name;
                if (sel instanceof MemberSelectTree) {
                    name = ((MemberSelectTree) sel).getIdentifier().toString();
                    receiver = ((MemberSelectTree) sel).getExpression();
                } else {
                    name = ((IdentifierTree) sel).getName().toString();
                }
                ns = ne - name.length();
                boolean ctor = name.equals("this") || name.equals("super");
                Map<String, Object> m = site(ctor ? "ctor" : "call", t, ns, ne, sym);
                if (!ctor) m.put("line", line(paren(ne)));
                argumentChecks(m, here, ctor ? null : receiver, t.getArguments(), sym);
                if (!ctor && !Boolean.TRUE.equals(m.get("err"))) {
                    String q = qualifier(here, receiver, sym);
                    if (q != null) m.put("qual", q);
                    if (receiver != null && receiver.getKind() == Tree.Kind.IDENTIFIER && ((IdentifierTree) receiver).getName().contentEquals("super")) m.put("super", true);
                }
                return super.visitMethodInvocation(t, v);
            }

            /** Whether this name is the method name of a call (no field site of its own). */
            boolean isCallName(Tree t) {
                Tree parent = getCurrentPath().getParentPath().getLeaf();
                return parent instanceof MethodInvocationTree && ((MethodInvocationTree) parent).getMethodSelect() == t;
            }

            @Override
            public Void visitNewClass(NewClassTree t, Void v) {
                TreePath here = getCurrentPath();
                Element sym = trees.getElement(here);
                Tree id = t.getIdentifier();
                Map<String, Object> m = site("ctor", t, start(id), end(id), sym);
                if (t.getClassBody() != null) m.put("anon", true);
                argumentChecks(m, here, t.getEnclosingExpression(), t.getArguments(), sym);
                return super.visitNewClass(t, v);
            }

            @Override
            public Void visitMemberSelect(MemberSelectTree t, Void v) {
                if (isCallName(t)) return super.visitMemberSelect(t, v);
                Element sym = trees.getElement(getCurrentPath());
                if (sym != null && (sym.getKind() == ElementKind.FIELD || sym.getKind() == ElementKind.ENUM_CONSTANT)) {
                    long ne = end(t);
                    Map<String, Object> m = site("field", t, ne - t.getIdentifier().length(), ne, sym);
                    argumentChecks(m, getCurrentPath(), t.getExpression(), List.of(), sym);
                } else if (sym == null || sym.asType() == null || sym.asType().getKind() == TypeKind.ERROR) {
                    // unresolved: a field, a type or a package, javac could not tell
                    TypeMirror tt = trees.getTypeMirror(getCurrentPath());
                    if (tt != null && tt.getKind() == TypeKind.ERROR && !(getCurrentPath().getParentPath().getLeaf() instanceof ImportTree)) {
                        long ne = end(t);
                        argumentChecks(site("unres", t, ne - t.getIdentifier().length(), ne, null), getCurrentPath(), t.getExpression(), List.of(), null);
                    }
                }
                return super.visitMemberSelect(t, v);
            }

            @Override
            public Void visitIdentifier(IdentifierTree t, Void v) {
                if (isCallName(t)) return null;
                Element sym = trees.getElement(getCurrentPath());
                if (sym != null && (sym.getKind() == ElementKind.FIELD || sym.getKind() == ElementKind.ENUM_CONSTANT)) {
                    Map<String, Object> m = site("field", t, start(t), end(t), sym);
                    argumentChecks(m, getCurrentPath(), null, List.of(), sym);
                } else if (sym == null || sym.asType() == null || sym.asType().getKind() == TypeKind.ERROR) {
                    TypeMirror tt = trees.getTypeMirror(getCurrentPath());
                    if (tt != null && tt.getKind() == TypeKind.ERROR) argumentChecks(site("unres", t, start(t), end(t), null), getCurrentPath(), null, List.of(), null);
                }
                return null;
            }

            @Override
            public Void visitMemberReference(MemberReferenceTree t, Void v) {
                Element sym = trees.getElement(getCurrentPath());
                long ne = end(t);
                String name = t.getMode() == MemberReferenceTree.ReferenceMode.NEW ? "new" : t.getName().toString();
                site("mref", t, ne - name.length(), ne, sym);
                return super.visitMemberReference(t, v);
            }

            @Override
            public Void visitClass(ClassTree t, Void v) {
                Element e = trees.getElement(getCurrentPath());
                if (members && e instanceof TypeElement) {
                    Map<String, Object> c = new LinkedHashMap<>();
                    c.put("name", binaryName(e));
                    c.put("start", off(start(t)));
                    c.put("line", line(start(t)));
                    c.put("end_line", line(end(t)));
                    c.put("missing", missingSuper((TypeElement) e, 0));
                    List<Map<String, Object>> ms = new ArrayList<>();
                    for (Element m : e.getEnclosedElements()) {
                        if (m instanceof TypeElement) continue;
                        Map<String, Object> x = new LinkedHashMap<>();
                        x.put("name", m.getSimpleName().toString());
                        x.put("kind", m.getKind().toString());
                        x.put("desc", descOf(m));
                        List<String> flags = new ArrayList<>();
                        for (Modifier mod : m.getModifiers()) flags.add(mod.toString());
                        x.put("flags", flags);
                        // a member no source declares: implicit (enum values, a
                        // record's accessors, a default constructor) or generated
                        // by a processor (Lombok)
                        x.put("tree", trees.getTree(m) != null);
                        ms.add(x);
                    }
                    c.put("members", ms);
                    classes.add(c);
                }
                return super.visitClass(t, v);
            }
        }
    }

    /** A JSON reader and writer, just enough for the protocol. */
    static final class Json {
        private final String s;
        private int i;

        Json(String s) {
            this.s = s;
        }

        Object value() {
            ws();
            char c = s.charAt(i);
            switch (c) {
                case '{': {
                    i++;
                    Map<String, Object> m = new LinkedHashMap<>();
                    ws();
                    if (s.charAt(i) == '}') {
                        i++;
                        return m;
                    }
                    while (true) {
                        ws();
                        String k = string();
                        ws();
                        expect(':');
                        m.put(k, value());
                        ws();
                        if (s.charAt(i) == ',') i++;
                        else {
                            expect('}');
                            return m;
                        }
                    }
                }
                case '[': {
                    i++;
                    List<Object> l = new ArrayList<>();
                    ws();
                    if (s.charAt(i) == ']') {
                        i++;
                        return l;
                    }
                    while (true) {
                        l.add(value());
                        ws();
                        if (s.charAt(i) == ',') i++;
                        else {
                            expect(']');
                            return l;
                        }
                    }
                }
                case '"':
                    return string();
                case 't':
                    i += 4;
                    return Boolean.TRUE;
                case 'f':
                    i += 5;
                    return Boolean.FALSE;
                case 'n':
                    i += 4;
                    return null;
                default: {
                    int st = i;
                    while (i < s.length() && "+-0123456789.eE".indexOf(s.charAt(i)) >= 0) i++;
                    String n = s.substring(st, i);
                    if (n.contains(".") || n.contains("e") || n.contains("E")) return Double.parseDouble(n);
                    return Long.parseLong(n);
                }
            }
        }

        private void ws() {
            while (i < s.length() && Character.isWhitespace(s.charAt(i))) i++;
        }

        private void expect(char c) {
            if (s.charAt(i) != c) throw new IllegalArgumentException("expected " + c + " at " + i);
            i++;
        }

        private String string() {
            expect('"');
            StringBuilder b = new StringBuilder();
            while (true) {
                char c = s.charAt(i++);
                if (c == '"') return b.toString();
                if (c != '\\') {
                    b.append(c);
                    continue;
                }
                char e = s.charAt(i++);
                switch (e) {
                    case 'n': b.append('\n'); break;
                    case 't': b.append('\t'); break;
                    case 'r': b.append('\r'); break;
                    case 'b': b.append('\b'); break;
                    case 'f': b.append('\f'); break;
                    case 'u': b.append((char) Integer.parseInt(s.substring(i, i + 4), 16)); i += 4; break;
                    default: b.append(e);
                }
            }
        }

        static String str(Object o) {
            StringBuilder b = new StringBuilder();
            write(b, o);
            return b.toString();
        }

        @SuppressWarnings("unchecked")
        static void write(StringBuilder b, Object o) {
            if (o == null) b.append("null");
            else if (o instanceof String) {
                String s = (String) o;
                b.append('"');
                for (int k = 0; k < s.length(); k++) {
                    char c = s.charAt(k);
                    if (c == '"' || c == '\\') b.append('\\').append(c);
                    else if (c < 0x20) b.append(String.format("\\u%04x", (int) c));
                    else b.append(c);
                }
                b.append('"');
            } else if (o instanceof Map) {
                b.append('{');
                boolean first = true;
                for (Map.Entry<String, Object> e : ((Map<String, Object>) o).entrySet()) {
                    if (!first) b.append(',');
                    first = false;
                    write(b, e.getKey());
                    b.append(':');
                    write(b, e.getValue());
                }
                b.append('}');
            } else if (o instanceof List) {
                b.append('[');
                boolean first = true;
                for (Object x : (List<Object>) o) {
                    if (!first) b.append(',');
                    first = false;
                    write(b, x);
                }
                b.append(']');
            } else b.append(o);
        }
    }
}
