import lombok.AllArgsConstructor;
import lombok.Data;

// Lombok processes local classes like any other.
class LombokLocal {
    void f() {
        @Data
        @AllArgsConstructor
        class Bean {
            private int order;
            private String name;
        }
        new Bean(1, "a").getName();
        new Bean(1); // expect
    }
}
