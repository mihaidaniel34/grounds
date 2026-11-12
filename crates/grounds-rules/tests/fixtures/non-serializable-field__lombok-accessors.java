import java.io.Serializable;
import lombok.Data;
import lombok.experimental.Accessors;

// @Accessors(chain = true) changes the setters Lombok generates in a way the
// model does not reproduce, here and in the nested class: their members are
// not all known. Whether DbConfig is Serializable does not depend on them
// (mybatis-plus GlobalConfig.dbConfig).
@Data
@Accessors(chain = true)
public class GlobalConfig implements Serializable {
    private DbConfig dbConfig; // expect

    @Data
    public static class DbConfig {
        private String schema;
    }
}
