import lombok.AllArgsConstructor;
import lombok.RequiredArgsConstructor;
import org.jetbrains.annotations.NotNull;

// Lombok takes the non-null annotations it knows (JetBrains' @NotNull among
// them) like its own @NonNull, and leaves fields whose name starts with `$`
// out of generated constructors.
@RequiredArgsConstructor(staticName = "ofCode")
class LombokResult {
    @NotNull
    private String code;
    private String message;

    static LombokResult ok() { return LombokResult.ofCode("00"); }
    static LombokResult none() { return new LombokResult(); } // expect
}

@AllArgsConstructor
class LombokHolder {
    String a;
    String b;
    String $cached;

    static LombokHolder h() { return new LombokHolder("a", "b"); }
    static LombokHolder h3() { return new LombokHolder("a", "b", "c"); } // expect
}
