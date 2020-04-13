use seed::Url;

#[derive(Debug, Clone, PartialEq)]
pub enum Page {
    Home,
    Callback,
}

impl Page {
    pub fn path(&self) -> Vec<&str> {
        match self {
            Page::Home => vec![],
            Page::Callback => vec!["callback"],
        }
    }

    pub fn from_url(url: Url) -> Option<Page> {
        Page::from_path(
            url.path
                .iter()
                .map(std::ops::Deref::deref)
                .collect::<Vec<&str>>(),
        )
    }

    pub fn from_path(path: Vec<&str>) -> Option<Page> {
        match path[..] {
            [] => Some(Page::Home),
            ["callback"] => Some(Page::Callback),
            _ => None,
        }
    }
}

impl From<Page> for Url {
    fn from(page: Page) -> Self {
        page.path().into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_page_from_path() {
        assert_eq!(Page::from_path(vec!["callback"]), Some(Page::Callback))
    }

    #[test]
    fn test_page_from_url() {
        let empty: Vec<&str> = vec![];
        let path: Vec<Url> = vec![
            Url::new(vec!["this", "should", "fail"]),
            Url::new(vec!["callback"]),
            Url::new(empty),
        ];
        assert_eq!(
            path.iter()
                .map(|x| Page::from_url(x.to_owned()))
                .collect::<Vec<Option<Page>>>(),
            vec![None, Some(Page::Callback), Some(Page::Home)]
        )
    }

    #[test]
    fn test_page_path() {
        let empty: Vec<&str> = vec![];
        let pages = vec![Page::Home, Page::Callback];
        assert_eq!(
            pages.iter().map(Page::path).collect::<Vec<Vec<&str>>>(),
            vec![empty, vec!["callback"]]
        )
    }
}
