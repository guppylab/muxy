use crate::{Client, ClientError};
use muxy_protocol::{
    CatalogPage, ErrorCode, MAX_PROJECTS, ProjectId, ProjectIntent, ProjectSessions, ReplyBody,
    RequestBody, SessionId,
};

impl Client {
    pub fn cancel_creation(
        &self,
        operation: muxy_protocol::OperationId,
    ) -> Result<(), ClientError> {
        match self.request(RequestBody::CancelCreation(operation))? {
            ReplyBody::CreationCancelled => Ok(()),
            body => Err(ClientError::UnexpectedReply(Box::new(body))),
        }
    }

    pub fn catalog_page(
        &self,
        after: Option<ProjectId>,
        revision: Option<u64>,
    ) -> Result<CatalogPage, ClientError> {
        match self.request(RequestBody::ReadCatalog { after, revision })? {
            ReplyBody::Catalog(page)
                if revision.is_none_or(|revision| page.revision == revision)
                    && page
                        .projects
                        .first()
                        .is_none_or(|project| after.is_none_or(|after| project.id > after)) =>
            {
                Ok(page)
            }
            body => Err(ClientError::UnexpectedReply(Box::new(body))),
        }
    }

    /// Fetches one revision, retrying a bounded number of times if it changes between pages.
    pub fn catalog(&self) -> Result<CatalogPage, ClientError> {
        for _ in 0..8 {
            match self.catalog_snapshot() {
                Err(ClientError::Server(error)) if error.code == ErrorCode::CatalogChanged => {}
                result => return result,
            }
        }
        Err(ClientError::Timeout)
    }

    fn catalog_snapshot(&self) -> Result<CatalogPage, ClientError> {
        let mut result = self.catalog_page(None, None)?;
        while let Some(after) = result.next {
            let page = self.catalog_page(Some(after), Some(result.revision))?;
            if page.server != result.server
                || page.home != result.home
                || page.next.is_some_and(|next| next <= after)
                || page
                    .projects
                    .first()
                    .is_some_and(|project| project.id <= after)
                || result.projects.len() + page.projects.len() > MAX_PROJECTS
            {
                return Err(ClientError::Protocol("invalid catalog pagination".into()));
            }
            result.projects.extend(page.projects);
            result.next = page.next;
        }
        Ok(result)
    }

    pub fn mutate_project(&self, intent: ProjectIntent) -> Result<u64, ClientError> {
        match self.request(RequestBody::MutateProject(intent))? {
            ReplyBody::ProjectMutated { revision } => Ok(revision),
            body => Err(ClientError::UnexpectedReply(Box::new(body))),
        }
    }

    pub fn project_sessions(
        &self,
        project: ProjectId,
        after: Option<SessionId>,
        revision: Option<u64>,
    ) -> Result<ProjectSessions, ClientError> {
        match self.request(RequestBody::ListProjectSessions {
            project,
            after,
            revision,
        })? {
            ReplyBody::ProjectSessions(page)
                if revision.is_none_or(|revision| page.revision == revision)
                    && page
                        .sessions
                        .iter()
                        .all(|session| session.info.project == project)
                    && page.sessions.first().is_none_or(|session| {
                        after.is_none_or(|after| session.info.id > after)
                    }) =>
            {
                Ok(page)
            }
            body => Err(ClientError::UnexpectedReply(Box::new(body))),
        }
    }
}
