use seed::Url;

#[derive(Debug, Clone, PartialEq)]
pub enum Page {
    Home,
}

impl Page {
    pub fn path(&self) -> Vec<&str> {
        match self {
            Page::Home => vec![],
        }
    }

    pub fn from_url(url: Url) -> Option<Page> {
        Page::from_path(
            url.hash()
                .unwrap_or(&String::default())
                .split("/")
                .collect::<Vec<&str>>(),
        )
    }

    pub fn from_path(path: Vec<&str>) -> Option<Page> {
        match path[..] {
            [""] => Some(Page::Home),
            _ => None,
        }
    }
}

impl From<Page> for Url {
    fn from(page: Page) -> Self {
        let url = Url::current();
        match page {
            Page::Home => url,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_page_from_path() {
        assert_eq!(Page::from_path(vec![""]), Some(Page::Home))
    }

    #[test]
    fn test_page_from_url() {
        let path: Vec<Url> = vec![Url::new().add_hash_path_part("thisshouldfail"), Url::new()];
        assert_eq!(
            path.iter()
                .map(|x| Page::from_url(x.to_owned()))
                .collect::<Vec<Option<Page>>>(),
            vec![None, Some(Page::Home)]
        )
    }

    #[test]
    fn test_page_path() {
        let empty: Vec<&str> = vec![];
        let pages = vec![Page::Home];
        assert_eq!(
            pages.iter().map(Page::path).collect::<Vec<Vec<&str>>>(),
            vec![empty]
        )
    }
}
