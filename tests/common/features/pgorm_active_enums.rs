use pgorm::entity::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, EnumIter, ActiveEnum)]
#[pgorm(rs_type = "String", db_type = "String(StringLen::N(1))")]
pub enum Category {
    #[pgorm(string_value = "B")]
    Big,
    #[pgorm(string_value = "S")]
    Small,
}

#[derive(Debug, Clone, PartialEq, Eq, EnumIter, ActiveEnum)]
#[pgorm(rs_type = "i32", db_type = "Integer")]
pub enum Color {
    #[pgorm(num_value = 0)]
    Black,
    #[pgorm(num_value = 1)]
    White,
}

#[derive(Debug, Clone, PartialEq, Eq, EnumIter, ActiveEnum)]
#[pgorm(rs_type = "String", db_type = "Enum", enum_name = "tea", display)]
pub enum Tea {
    #[pgorm(string_value = "EverydayTea")]
    EverydayTea,
    #[pgorm(string_value = "BreakfastTea")]
    BreakfastTea,
}

#[derive(Debug, Clone, PartialEq, Eq, EnumIter, ActiveEnum, Copy)]
#[pgorm(rs_type = "String", db_type = "Enum", enum_name = "media_type")]
pub enum MediaType {
    #[pgorm(string_value = "UNKNOWN")]
    Unknown,
    #[pgorm(string_value = "BITMAP")]
    Bitmap,
    #[pgorm(string_value = "DRAWING")]
    Drawing,
    #[pgorm(string_value = "AUDIO")]
    Audio,
    #[pgorm(string_value = "VIDEO")]
    Video,
    #[pgorm(string_value = "MULTIMEDIA")]
    Multimedia,
    #[pgorm(string_value = "OFFICE")]
    Office,
    #[pgorm(string_value = "TEXT")]
    Text,
    #[pgorm(string_value = "EXECUTABLE")]
    Executable,
    #[pgorm(string_value = "ARCHIVE")]
    Archive,
    #[pgorm(string_value = "3D")]
    _3D,
}

#[derive(Debug, Clone, PartialEq, Eq, EnumIter, ActiveEnum)]
#[pgorm(rs_type = "String", db_type = "Enum", enum_name = "tea", display)]
pub enum DisplayTea {
    #[pgorm(string_value = "EverydayTea", display_value = "Everyday")]
    EverydayTea,
    #[pgorm(string_value = "BreakfastTea", display_value = "Breakfast")]
    BreakfastTea,
}
