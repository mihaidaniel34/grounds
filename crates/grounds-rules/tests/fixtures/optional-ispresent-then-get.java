import java.util.Optional;
class A {
    void m(Optional<String> o) {
        if (o.isPresent()) { // expect
            System.out.println(o.get());
        }
        o.ifPresent(System.out::println);
        if (o.isPresent()) { System.out.println(o.get()); } else { System.out.println("none"); }
    }
}
