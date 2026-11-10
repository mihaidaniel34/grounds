// The capture of a wildcard for a type parameter with several bounds has all
// of them (JLS 4.9, 5.1.10): members of the second bound are there.
abstract class Root { Object root() { return null; } }
interface Bytecode { default Object getBytecodeNode() { return null; } }

class Nodes<N extends Root & Bytecode> {
    N getNode(int i) { return null; }
    Object own(N n) { return n.getBytecodeNode(); }
}

class SingleBound<R extends Root> {
    R get() { return null; }
}

class UseNodes {
    Object f(Nodes<?> nodes) { return nodes.getNode(0).getBytecodeNode(); }
    Object g(Nodes<?> nodes) { return nodes.getNode(0).root(); }
    Object h(SingleBound<?> s) { return s.get().getBytecodeNode(); } // expect
}
