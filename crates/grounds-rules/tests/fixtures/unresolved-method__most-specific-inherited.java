// A method that reaches a class through several supertypes: the declaration
// that overrides the others, or among abstract ones the one with the most
// specific return type, is the one a call selects (JLS 8.4.8, 15.12.2.5).
interface Model { Model setPrefix(String p); }
interface Prefixed<R> { R setPrefix(String p); }
interface OntModel extends Model, Prefixed<OntModel> { OntModel setId(String id); }

interface Data { }
interface Manager { Data data(); }
class Cache implements Data { int check(String p, int o) { return 0; } }
abstract class AbstractManager implements Manager { public Cache data() { return new Cache(); } }
abstract class HolderManager<H> extends AbstractManager { }
class UserManager extends HolderManager<String> implements Manager { }

class MostSpecificUse {
    void f(OntModel m) { m.setPrefix("x").setId("y"); }
    int g(UserManager u) { return u.data().check("x", 1); }
    void h(OntModel m) { m.setPrefix("x").setName("y"); } // expect
    int k(UserManager u) { return u.data().clear(); } // expect
    int l(Manager m) { return m.data().check("x", 1); } // expect
}
