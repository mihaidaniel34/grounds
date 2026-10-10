interface Shape {}
final class Point {}
class Base {}
class Sub extends Base {}
class A {
    boolean m(String s, Point p, Base b, Shape sh, Object o) {
        if (s instanceof Number) return true; // expect
        if (p instanceof Shape) return true; // expect
        if (b instanceof Sub) return true;
        if (sh instanceof Base) return true;
        return o instanceof String;
    }
}
interface SF {}
final class Docs implements SF {}
final class PE implements SF {}
class Parse {
    java.util.stream.Stream<SF> m(java.util.stream.Stream<String> in) {
        return in.map(s -> { try { return (SF) new Docs(); } catch (RuntimeException e) { return new PE(); } })
                 .map(sf -> sf instanceof Docs ? sf : null);
    }
}
