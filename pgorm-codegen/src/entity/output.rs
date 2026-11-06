#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriterOutput {
    pub files: Vec<OutputFile>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputFile {
    pub name: String,
    pub content: String,
}
