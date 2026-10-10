import jakarta.inject.Inject;
class A {
    @Inject final Object dep = null; // expect
    @Inject Object ok;
}
