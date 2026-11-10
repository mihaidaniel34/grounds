import java.util.List;

// A method reference to an overloaded method denotes the overload whose
// parameters take the function type's (JLS 15.13.1), not the first of the
// right arity.
class ConvA { }
class ConvB { }
class ResA { void x() { } }
class ResB { void y() { } }

class Conv {
    static ResA conv(ConvA a) { return null; }
    static ResB conv(ConvB b) { return null; }
}

class UseConv {
    void f(List<ConvB> l) { l.stream().map(Conv::conv).forEach(r -> r.y()); }
    void g(List<ConvA> l) { l.stream().map(Conv::conv).forEach(r -> r.x()); }
    void h(List<ConvA> l) { l.stream().map(Conv::conv).forEach(r -> r.y()); } // expect
}

class MrGood { void ok() { } }
class MrBad { void other() { } }

class MostSpecificRef {
    static MrGood m(String x) { return null; }
    static MrBad m(Object x) { return null; }

    void test(List<String> xs) { xs.stream().map(MostSpecificRef::m).forEach(x -> x.ok()); }
    void wrong(List<String> xs) { xs.stream().map(MostSpecificRef::m).forEach(x -> x.nonexistent()); } // expect
}

class RefHelped { void ok() { } }
interface RefImpl { private RefHelped help() { return null; } default void use() { help(); } }
interface RefApi { RefHelped help(); }
interface RefBoth extends RefImpl, RefApi { }

class PrivateNotARefCandidate {
    void test(List<RefBoth> xs) { xs.stream().map(RefBoth::help).forEach(x -> x.ok()); }
    void wrong(List<RefBoth> xs) { xs.stream().map(RefBoth::help).forEach(x -> x.nonexistent()); } // expect
}
