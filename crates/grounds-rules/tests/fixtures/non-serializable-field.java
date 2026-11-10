import java.io.Serializable;
import java.util.List;
final class NotSer { }
class A implements Serializable {
    private NotSer bad; // expect
    private transient NotSer ok;
    private List<String> iface;
    private String s;
    private Thread thread; // expect
}
class Holder implements java.io.Serializable {
    private Object payload;
}
// Serializable only through a library superclass: not written to be serialized
class Bag extends java.util.ArrayList<String> {
    private NotSer helper;
}
// Serializable through a project superclass: the subclass's fields count
class Base implements Serializable { }
class Derived extends Base {
    private NotSer bad; // expect
}
// CDI: a normal-scoped bean is injected as a serializable client proxy
@jakarta.enterprise.context.ApplicationScoped
class Probe { }
class Plain { }
@jakarta.faces.view.ViewScoped
class ViewBean implements Serializable {
    @jakarta.inject.Inject
    private Probe probe;
    @jakarta.inject.Inject
    private Plain injected;
    private Probe byConstructor;
    private Plain notInjected; // expect
    @jakarta.inject.Inject
    ViewBean(Probe p) { byConstructor = p; }
}
// not a passivating CDI bean: nothing checks what is injected
class Session implements Serializable {
    @jakarta.inject.Inject
    private Plain helper; // expect
}
// a normal-scoped type built directly is a plain object, not a proxy
class DirectHolder implements Serializable {
    private Probe probe = new Probe(); // expect
}
