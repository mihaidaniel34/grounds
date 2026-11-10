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
