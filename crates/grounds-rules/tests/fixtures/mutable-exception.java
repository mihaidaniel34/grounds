class MyException extends Exception {
    private String detail; // expect
    private final int code = 1;
}
@lombok.Value
class ValueException extends RuntimeException { String code; }
